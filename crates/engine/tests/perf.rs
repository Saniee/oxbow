//! Timings against a real repository, for finding slow spots. Ignored by default:
//! `OXBOW_PERF_REPO=path cargo test -p oxbow-engine --test perf -- --ignored --nocapture`

use std::time::Instant;

use oxbow_engine::{Git, Settings, Snapshot, queries, status};

#[test]
#[ignore]
fn snapshot_load_times() {
    let path = std::env::var("OXBOW_PERF_REPO").expect("set OXBOW_PERF_REPO");
    let git = Git::open(&path).unwrap();
    let settings = Settings::default();
    // Each query on its own, then the whole (parallel) snapshot, a few times.
    let time = |name: &str, f: &dyn Fn()| {
        let start = Instant::now();
        f();
        println!("{name:>16}: {:>6.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    };
    time("status", &|| drop(status::status(&git).unwrap()));
    time("local branches", &|| drop(queries::local_branches(&git, settings.branch_sort).unwrap()));
    time("remotes", &|| drop(queries::remote_branches(&git, &queries::remotes(&git).unwrap()).unwrap()));
    time("tags", &|| drop(queries::tags(&git).unwrap()));
    time("worktrees", &|| drop(queries::worktrees(&git).unwrap()));
    time("log", &|| drop(queries::log(&git, &queries::LogQuery { limit: settings.commit_limit, ..Default::default() }).unwrap()));
    time("reflog 1000", &|| drop(queries::reflog(&git, 1000).unwrap()));
    time("unpushed", &|| drop(queries::unpushed(&git).unwrap()));
    time("stashes", &|| drop(queries::stashes(&git).unwrap()));
    time("bisect", &|| drop(queries::bisect_state(&git).unwrap()));
    for n in 1..=5 {
        time(&format!("snapshot #{n}"), &|| drop(Snapshot::load(&git, &settings, None).unwrap()));
    }
}

#[test]
#[ignore]
fn one_traced_snapshot() {
    let git = Git::open(std::env::var("OXBOW_PERF_REPO").expect("set OXBOW_PERF_REPO")).unwrap();
    drop(Snapshot::load(&git, &Settings::default(), None).unwrap());
    let start = Instant::now();
    drop(Snapshot::load(&git, &Settings::default(), None).unwrap());
    println!("snapshot: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
}
