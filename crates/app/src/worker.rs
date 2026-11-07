//! Runs git off the UI thread. Actions run one at a time, in order, each followed by a fresh snapshot; detail
//! loads (a commit's patch, a file's diff) run on their own threads so they never wait behind a fetch.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui;
use oxbow_engine::{CommandLog, Git, Result, Settings, Snapshot};

pub type Task = Box<dyn FnOnce(&Git) -> Result<()> + Send>;

/// Something the user asked for, with the title shown while it runs and in the command log.
pub struct Action {
    pub title: String,
    pub task: Task,
}

pub fn action(title: impl Into<String>, task: impl FnOnce(&Git) -> Result<()> + Send + 'static) -> Action {
    Action { title: title.into(), task: Box::new(task) }
}

enum Job {
    Refresh,
    Run(Action),
}

pub enum Event<D> {
    Snapshot(std::result::Result<Snapshot, String>),
    /// An action finished; `error` is set when it failed.
    Done { title: String, error: Option<String> },
    Detail { generation: u64, result: std::result::Result<D, String> },
}

/// What the snapshot is loaded with; the UI changes it, the worker reads it before each load.
#[derive(Default, Clone)]
pub struct LoadOptions {
    pub settings: Settings,
    pub search: Option<String>,
}

pub struct Worker<D> {
    pub git: Git,
    pub log: Arc<CommandLog>,
    /// None once dropping: closing the channel ends the worker thread.
    jobs: Option<Sender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// One clone per detail load still running.
    detail_loads: Arc<()>,
    events_tx: Sender<Event<D>>,
    events: Receiver<Event<D>>,
    ctx: egui::Context,
    refresh_queued: Arc<AtomicBool>,
    /// The action running now, for the status line.
    pub running: Arc<Mutex<Option<String>>>,
    pub options: Arc<Mutex<LoadOptions>>,
}

impl<D: Send + 'static> Worker<D> {
    pub fn new(git: Git, ctx: egui::Context, options: LoadOptions) -> Self {
        let log = Arc::new(CommandLog::default());
        let git = git.with_log(log.clone());
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (events_tx, events) = mpsc::channel();
        let refresh_queued = Arc::new(AtomicBool::new(false));
        let running = Arc::new(Mutex::new(None));
        let options = Arc::new(Mutex::new(options));
        let thread = {
            let (git, log, tx, ctx) = (git.clone(), log.clone(), events_tx.clone(), ctx.clone());
            let (queued, running, options) = (refresh_queued.clone(), running.clone(), options.clone());
            std::thread::Builder::new()
                .name("oxbow-git".into())
                .spawn(move || {
                    let load = |tx: &Sender<Event<D>>| {
                        queued.store(false, Ordering::SeqCst);
                        let opts = options.lock().unwrap().clone();
                        let snap = Snapshot::load(&git, &opts.settings, opts.search.as_deref()).map_err(|e| e.to_string());
                        let _ = tx.send(Event::Snapshot(snap));
                        ctx.request_repaint();
                    };
                    for job in job_rx {
                        match job {
                            Job::Refresh => load(&tx),
                            Job::Run(Action { title, task }) => {
                                *running.lock().unwrap() = Some(title.clone());
                                ctx.request_repaint();
                                let id = log.start(&title);
                                let result = task(&git);
                                log.finish(id, result.is_ok());
                                *running.lock().unwrap() = None;
                                let _ = tx.send(Event::Done { title, error: result.err().map(|e| e.to_string()) });
                                load(&tx);
                            }
                        }
                    }
                })
                .expect("could not start the git worker")
        };
        let worker = Self { git, log, jobs: Some(jobs), thread: Some(thread), detail_loads: Arc::new(()), events_tx, events, ctx, refresh_queued, running, options };
        worker.refresh();
        worker
    }

    /// Reloads the snapshot, unless a reload is already waiting.
    pub fn refresh(&self) {
        if !self.refresh_queued.swap(true, Ordering::SeqCst)
            && let Some(jobs) = &self.jobs
        {
            let _ = jobs.send(Job::Refresh);
        }
    }

    pub fn run(&self, action: Action) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Job::Run(action));
        }
    }

    /// Loads a detail on its own thread; the result comes back tagged with `generation`.
    pub fn load_detail(&self, generation: u64, load: impl FnOnce(&Git) -> std::result::Result<D, String> + Send + 'static) {
        let (git, tx, ctx, token) = (self.git.clone(), self.events_tx.clone(), self.ctx.clone(), self.detail_loads.clone());
        std::thread::spawn(move || {
            let _token = token;
            let result = load(&git);
            let _ = tx.send(Event::Detail { generation, result });
            ctx.request_repaint();
        });
    }

    pub fn poll(&self) -> Vec<Event<D>> {
        self.events.try_iter().collect()
    }

    pub fn running(&self) -> Option<String> {
        self.running.lock().unwrap().clone()
    }
}

impl<D> Drop for Worker<D> {
    /// Waits for git to finish what it is doing (a push, say) rather than leaving it running on its own.
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let start = std::time::Instant::now();
        while Arc::strong_count(&self.detail_loads) > 1 && start.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
