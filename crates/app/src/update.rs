//! Updates from Oxbow's GitHub Releases: find a newer version, download the file for how this copy was installed,
//! check it against the release's SHA256SUMS, and install it.
//!
//! - Bug-fix releases (same major and minor version, like 0.1.1 to 0.1.2) are downloaded on their own. The Windows
//!   installer then runs when Oxbow closes; an AppImage is replaced at once and the new one starts next time.
//! - Feature releases (0.1 to 0.2, or a new major version) are only offered: nothing is downloaded until asked.
//! - Copies that can't replace themselves (a .deb, the .zip or .tar.gz, or an all-users install that would need an
//!   administrator) get the offer too, with a link to the release when Oxbow can't install it.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use eframe::egui;
use sha2::{Digest, Sha256};

/// This build's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "https://api.github.com/repos/Saniee/oxbow/releases?per_page=30";
/// Oxbow's release tags; the repository also has the SlothGit extension's (0.0.1 … 1.0.0).
const TAG_PREFIX: &str = "oxbow-v";
const SUMS: &str = "SHA256SUMS";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// `1.2.3`; anything else (a pre-release like `1.2.3-beta.1`, or junk) is None.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.').map(|p| if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) { p.parse().ok() } else { None });
        let v = Version { major: parts.next()??, minor: parts.next()??, patch: parts.next()?? };
        parts.next().is_none().then_some(v)
    }

    pub fn current() -> Self {
        Self::parse(CURRENT).expect("the crate version is major.minor.patch")
    }

    /// A newer bug-fix release of `self`: same major and minor version.
    pub fn is_fix_for(self, current: Version) -> bool {
        self.major == current.major && self.minor == current.minor && self.patch > current.patch
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    /// The release's page on GitHub, with its notes and every download.
    pub page: String,
    /// File name and download URL.
    pub assets: Vec<(String, String)>,
}

impl Release {
    fn asset(&self, name: &str) -> Option<&str> {
        self.assets.iter().find(|a| a.0 == name).map(|a| a.1.as_str())
    }
}

/// Oxbow's published releases in GitHub's answer, leaving out drafts, pre-releases and other tags.
pub fn parse_releases(json: &str) -> Result<Vec<Release>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("GitHub's answer couldn't be read: {e}"))?;
    let list = value.as_array().ok_or("GitHub's answer wasn't a list of releases")?;
    Ok(list
        .iter()
        .filter(|r| !r["draft"].as_bool().unwrap_or(true) && !r["prerelease"].as_bool().unwrap_or(true))
        .filter_map(|r| {
            let version = Version::parse(r["tag_name"].as_str()?.strip_prefix(TAG_PREFIX)?)?;
            let assets = r["assets"]
                .as_array()?
                .iter()
                .filter_map(|a| Some((a["name"].as_str()?.to_owned(), a["browser_download_url"].as_str()?.to_owned())))
                .collect();
            Some(Release { version, page: r["html_url"].as_str()?.to_owned(), assets })
        })
        .collect())
}

/// The newest release that's newer than `current`.
pub fn newest(releases: Vec<Release>, current: Version) -> Option<Release> {
    releases.into_iter().filter(|r| r.version > current).max_by_key(|r| r.version)
}

/// The SHA-256 that SHA256SUMS (`sha256sum`'s output) gives for `name`.
pub fn checksum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

/// Copies `from` into `dest`, and keeps it only if its SHA-256 is `expected`. It's written next to `dest` first, so
/// `dest` is either the whole checked file or untouched.
pub fn save_checked(mut from: impl Read, dest: &Path, expected: &str) -> io::Result<()> {
    let part = dest.with_file_name(format!(".{}.part", dest.file_name().and_then(|n| n.to_str()).unwrap_or("download")));
    let result = (|| {
        let mut file = std::fs::File::create(&part)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0; 64 * 1024];
        loop {
            let n = from.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
        }
        file.sync_all()?;
        drop(file);
        let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != expected {
            return Err(io::Error::other(format!("the download's checksum is {got}, but the release says {expected}")));
        }
        std::fs::rename(&part, dest)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// How this copy of Oxbow was installed, which decides how it's updated.
#[derive(Clone, Debug, PartialEq)]
pub enum Install {
    /// By the Windows installer, into a folder Oxbow can write to (one for the current user).
    Installer,
    /// By the Windows installer for all users: updating asks for an administrator.
    AdminInstaller,
    /// An AppImage at this path, in a folder Oxbow can write to.
    AppImage(PathBuf),
    /// A .deb, the .zip or .tar.gz, or a build: Oxbow can't update it, only point to the release.
    Manual,
}

impl Install {
    pub fn detect() -> Self {
        if let Some(app_image) = std::env::var_os("APPIMAGE").map(PathBuf::from) {
            return match app_image.parent() {
                Some(dir) if writable(dir) => Install::AppImage(app_image),
                _ => Install::Manual,
            };
        }
        let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_owned)) else { return Install::Manual };
        if cfg!(windows) && dir.join("unins000.exe").is_file() {
            return if writable(&dir) { Install::Installer } else { Install::AdminInstaller };
        }
        Install::Manual
    }

    /// The release file this kind of install updates from.
    fn asset_name(&self, version: Version) -> Option<String> {
        match self {
            Install::Installer | Install::AdminInstaller => Some(format!("oxbow-{version}-windows-x64-setup.exe")),
            Install::AppImage(_) => Some(format!("oxbow-{version}-x86_64.AppImage")),
            Install::Manual => None,
        }
    }

    /// Whether a bug-fix release can be put in place without asking: no administrator, no package manager.
    fn installs_quietly(&self) -> bool {
        matches!(self, Install::Installer | Install::AppImage(_))
    }
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".oxbow-write-test-{}", std::process::id()));
    let ok = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .user_agent(format!("oxbow/{CURRENT}"))
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .build(),
    )
}

fn get(agent: &ureq::Agent, url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    agent.get(url).header("Accept", "application/vnd.github+json").call().map_err(|e| format!("couldn't reach GitHub: {e}"))
}

fn fetch_newest() -> Result<Option<Release>, String> {
    let agent = agent();
    let json = get(&agent, RELEASES)?.body_mut().read_to_string().map_err(|e| format!("couldn't read GitHub's answer: {e}"))?;
    Ok(newest(parse_releases(&json)?, Version::current()))
}

/// Downloads the release's file for `install` into `dir` and checks it. Returns its path.
fn download(release: &Release, install: &Install, dir: &Path) -> Result<PathBuf, String> {
    let name = install.asset_name(release.version).ok_or("this copy of Oxbow can't update itself")?;
    let url = release.asset(&name).ok_or_else(|| format!("the release has no {name}"))?;
    let sums_url = release.asset(SUMS).ok_or("the release has no SHA256SUMS")?;
    let agent = agent();
    let sums = get(&agent, sums_url)?.body_mut().read_to_string().map_err(|e| format!("couldn't read SHA256SUMS: {e}"))?;
    let expected = checksum_for(&sums, &name).ok_or_else(|| format!("SHA256SUMS doesn't list {name}"))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't make {}: {e}", dir.display()))?;
    let dest = dir.join(&name);
    let body = get(&agent, url)?.into_body().into_reader();
    save_checked(body, &dest, &expected).map_err(|e| format!("couldn't download {name}: {e}"))?;
    Ok(dest)
}

/// Where the Windows installer is downloaded to.
fn download_dir() -> PathBuf {
    std::env::temp_dir().join("oxbow-update")
}

/// Puts a downloaded AppImage in place of the running one. The running copy keeps going from the old file.
fn replace_app_image(new: &Path, app_image: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(new, app_image)
}

/// Runs the downloaded Windows installer without its pages. With `reopen`, it starts Oxbow again afterwards, on
/// that folder when there is one (`Some(None)`: the project list).
fn run_installer(installer: &Path, reopen: Option<Option<&Path>>) -> io::Result<()> {
    let mut command = std::process::Command::new(installer);
    command.args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS"]);
    if let Some(folder) = reopen {
        command.arg("/RELAUNCH=yes");
        if let Some(folder) = folder {
            command.arg(format!("/OPEN={}", folder.display()));
        }
    }
    command.spawn().map(drop)
}

/// Where updating stands, for the toolbar's notice and the Settings window.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    /// A newer release, offered. `can_install`: Oxbow can install it (otherwise it links to the release).
    Available { release: Release, can_install: bool },
    Downloading(Release),
    /// The Windows installer is downloaded and runs when Oxbow closes, or now with Restart.
    Ready { release: Release, installer: PathBuf },
    /// The AppImage was replaced; the new version starts next time.
    Installed(Release),
    Failed(String),
}

enum Done {
    Checked(Result<Option<Release>, String>),
    /// The file was downloaded (and an AppImage put in place); `now`: install and restart straight away.
    Downloaded { release: Release, result: Result<PathBuf, String>, now: bool },
}

pub struct Updater {
    ctx: egui::Context,
    install: Install,
    pub state: State,
    /// The last check was asked for in Settings: say when there's nothing new, or it failed.
    manual: bool,
    /// Install bug-fix releases without asking.
    pub auto_fixes: bool,
    /// The notice was closed for this version.
    dismissed: Option<Version>,
    pending: Option<Receiver<Done>>,
    /// Asked to close Oxbow (for Restart), after starting the new version's install.
    pub close: bool,
    installer_started: bool,
}

impl Updater {
    pub fn new(ctx: egui::Context, auto_fixes: bool) -> Self {
        Self { ctx, install: Install::detect(), state: State::Idle, manual: false, auto_fixes, dismissed: None, pending: None, close: false, installer_started: false }
    }

    fn spawn(&mut self, job: impl FnOnce() -> Done + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
        self.pending = Some(rx);
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn check(&mut self, manual: bool) {
        if self.busy() || matches!(self.state, State::Ready { .. } | State::Installed(_)) {
            return;
        }
        self.manual = manual;
        self.state = State::Checking;
        self.spawn(|| Done::Checked(fetch_newest()));
    }

    /// Download (and for an AppImage, put in place) `release`. `now`: then install and restart.
    fn start_download(&mut self, release: Release, now: bool) {
        let install = self.install.clone();
        // How a download goes is told in the toolbar, whoever started it.
        self.manual = false;
        self.state = State::Downloading(release.clone());
        self.spawn(move || {
            let result = match &install {
                Install::AppImage(path) => {
                    let dir = path.parent().unwrap_or(Path::new("."));
                    download(&release, &install, dir).and_then(|new| replace_app_image(&new, path).map(|()| path.clone()).map_err(|e| format!("couldn't replace {}: {e}", path.display())))
                }
                _ => download(&release, &install, &download_dir()),
            };
            Done::Downloaded { release, result, now }
        });
    }

    /// Takes in what a background check or download found.
    pub fn poll(&mut self, folder: Option<&Path>) {
        let Some(rx) = &self.pending else { return };
        let Ok(done) = rx.try_recv() else { return };
        self.pending = None;
        match done {
            Done::Checked(Err(e)) => self.state = if self.manual { State::Failed(e) } else { State::Idle },
            Done::Checked(Ok(None)) => {
                // Nothing newer: an installer downloaded before has done its job (or is out of date).
                let _ = std::fs::remove_dir_all(download_dir());
                self.state = if self.manual { State::UpToDate } else { State::Idle };
            }
            Done::Checked(Ok(Some(release))) => {
                if self.auto_fixes && self.install.installs_quietly() && release.version.is_fix_for(Version::current()) {
                    self.start_download(release, false);
                } else {
                    let can_install = self.install.asset_name(release.version).is_some_and(|n| release.asset(&n).is_some() && release.asset(SUMS).is_some());
                    self.state = State::Available { release, can_install };
                }
            }
            Done::Downloaded { release, result: Err(e), .. } => self.state = State::Failed(format!("Oxbow {} couldn't be downloaded: {e}", release.version)),
            Done::Downloaded { release, result: Ok(file), now } => {
                self.state = match self.install {
                    Install::AppImage(_) => State::Installed(release),
                    _ => State::Ready { release, installer: file },
                };
                if now {
                    self.restart(folder);
                }
            }
        }
    }

    /// Update now: download, install and start the new version.
    pub fn update_now(&mut self, folder: Option<&Path>) {
        match self.state.clone() {
            State::Available { release, can_install: true } => self.start_download(release, true),
            State::Ready { .. } | State::Installed(_) => self.restart(folder),
            _ => {}
        }
    }

    /// Installs what's downloaded and starts the new version on `folder`, then asks Oxbow to close.
    fn restart(&mut self, folder: Option<&Path>) {
        let started = match &self.state {
            State::Ready { installer, .. } => run_installer(installer, Some(folder)).map_err(|e| format!("couldn't start the installer: {e}")),
            State::Installed(_) => match &self.install {
                Install::AppImage(path) => {
                    let mut command = std::process::Command::new(path);
                    command.args(folder);
                    command.spawn().map(drop).map_err(|e| format!("couldn't start the new Oxbow: {e}"))
                }
                _ => Ok(()),
            },
            _ => return,
        };
        match started {
            Ok(()) => {
                self.installer_started = true;
                self.close = true;
            }
            Err(e) => self.state = State::Failed(e),
        }
    }

    /// Oxbow is closing: a downloaded bug-fix installs now, quietly, without starting Oxbow again.
    pub fn on_exit(&mut self) {
        if let (State::Ready { installer, .. }, false) = (&self.state, self.installer_started) {
            let _ = run_installer(installer, None);
            self.installer_started = true;
        }
    }

    /// The notice in the toolbar, when there's something to say there.
    pub fn notice(&mut self, ui: &mut egui::Ui, folder: Option<&Path>) {
        match &self.state {
            State::Available { release, .. } if self.dismissed == Some(release.version) => return,
            State::Available { .. } | State::Downloading(_) | State::Ready { .. } | State::Installed(_) => {}
            // A failed check asked for in Settings is told there.
            State::Failed(_) if !self.manual => {}
            _ => return,
        }
        ui.separator();
        match self.state.clone() {
            State::Available { release, can_install } => {
                ui.label(accent(ui, format!("Oxbow {} is available", release.version)));
                if can_install && ui.button("Update now").on_hover_text("Download it, install it and start Oxbow again").clicked() {
                    self.update_now(folder);
                }
                ui.hyperlink_to(if can_install { "What's new" } else { "Download" }, &release.page);
                if ui.small_button("✕").on_hover_text("Not now").clicked() {
                    self.dismissed = Some(release.version);
                }
            }
            State::Downloading(release) => {
                ui.spinner();
                ui.label(format!("Downloading Oxbow {}…", release.version));
            }
            State::Ready { release, .. } => {
                ui.label(accent(ui, format!("Oxbow {} installs when you close Oxbow", release.version)));
                if ui.button("Restart now").clicked() {
                    self.restart(folder);
                }
                ui.hyperlink_to("What's new", &release.page);
            }
            State::Installed(release) => {
                ui.label(accent(ui, format!("Oxbow {} is installed and starts next time", release.version)));
                if ui.button("Restart now").clicked() {
                    self.restart(folder);
                }
                ui.hyperlink_to("What's new", &release.page);
            }
            State::Failed(e) => {
                ui.label(egui::RichText::new(format!("Updating failed: {e}")).color(ui.visuals().warn_fg_color));
                if ui.small_button("✕").clicked() {
                    self.state = State::Idle;
                }
            }
            _ => {}
        }
    }

    /// One line for the Settings window: what the last check found.
    pub fn status(&self) -> String {
        match &self.state {
            State::Idle => format!("This is Oxbow {CURRENT}."),
            State::Checking => "Checking…".into(),
            State::UpToDate => format!("Oxbow {CURRENT} is the newest version."),
            State::Available { release, can_install: true } => format!("Oxbow {} is available; Update now is in the toolbar.", release.version),
            State::Available { release, can_install: false } => format!("Oxbow {} is available; this copy is updated by downloading it.", release.version),
            State::Downloading(release) => format!("Downloading Oxbow {}…", release.version),
            State::Ready { release, .. } => format!("Oxbow {} is downloaded and installs when you close Oxbow.", release.version),
            State::Installed(release) => format!("Oxbow {} is installed and starts next time.", release.version),
            State::Failed(e) => format!("Updating failed: {e}"),
        }
    }
}

/// Text in the theme's accent colour (the selection's), for the update notice.
fn accent(ui: &egui::Ui, text: String) -> egui::RichText {
    egui::RichText::new(text).color(ui.visuals().selection.stroke.color).strong()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn versions_compare_by_number_and_tell_fixes_apart() {
        assert!(v("0.1.10") > v("0.1.9"));
        assert!(v("0.2.0") > v("0.1.99"));
        assert!(v("1.0.0") > v("0.9.9"));
        assert!(v("0.1.2").is_fix_for(v("0.1.1")));
        assert!(!v("0.2.0").is_fix_for(v("0.1.1")), "a minor release is a feature release");
        assert!(!v("1.1.2").is_fix_for(v("0.1.1")));
        assert!(!v("0.1.1").is_fix_for(v("0.1.1")));
        for junk in ["", "1", "1.2", "1.2.3.4", "1.2.3-beta.1", "v1.2.3", "1.x.3", "1..3"] {
            assert_eq!(Version::parse(junk), None, "{junk:?}");
        }
        assert_eq!(Version::current().to_string(), CURRENT);
    }

    const ANSWER: &str = r#"[
        {"tag_name": "oxbow-v0.3.0-beta.1", "draft": false, "prerelease": true, "html_url": "https://x/beta", "assets": []},
        {"tag_name": "oxbow-v0.2.1", "draft": true, "prerelease": false, "html_url": "https://x/draft", "assets": []},
        {"tag_name": "oxbow-v0.2.0", "draft": false, "prerelease": false, "html_url": "https://x/0.2.0", "assets": [
            {"name": "oxbow-0.2.0-windows-x64-setup.exe", "browser_download_url": "https://x/setup.exe"},
            {"name": "SHA256SUMS", "browser_download_url": "https://x/sums"}]},
        {"tag_name": "oxbow-v0.1.2", "draft": false, "prerelease": false, "html_url": "https://x/0.1.2", "assets": []},
        {"tag_name": "1.0.0", "draft": false, "prerelease": false, "html_url": "https://x/slothgit", "assets": []}
    ]"#;

    #[test]
    fn only_oxbows_published_releases_count() {
        let releases = parse_releases(ANSWER).unwrap();
        let versions: Vec<String> = releases.iter().map(|r| r.version.to_string()).collect();
        assert_eq!(versions, ["0.2.0", "0.1.2"], "no drafts, pre-releases or SlothGit tags");
        let newest = newest(releases.clone(), v("0.1.1")).unwrap();
        assert_eq!(newest.page, "https://x/0.2.0");
        assert_eq!(newest.asset("SHA256SUMS"), Some("https://x/sums"));
        assert_eq!(super::newest(releases.clone(), v("0.2.0")), None, "nothing newer");
        assert_eq!(super::newest(releases, v("0.5.0")), None, "a build newer than any release");
        assert!(parse_releases("{\"message\": \"API rate limit exceeded\"}").is_err());
        assert!(parse_releases("not json").is_err());
    }

    #[test]
    fn checksums_are_read_the_way_sha256sum_writes_them() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!("{a}  oxbow-0.2.0-x86_64.AppImage\n{b} *oxbow-0.2.0-windows-x64-setup.exe\nshort  other.txt\n");
        assert_eq!(checksum_for(&sums, "oxbow-0.2.0-x86_64.AppImage"), Some(a));
        assert_eq!(checksum_for(&sums, "oxbow-0.2.0-windows-x64-setup.exe"), Some("b".repeat(64)));
        assert_eq!(checksum_for(&sums, "other.txt"), None, "not a SHA-256");
        assert_eq!(checksum_for(&sums, "oxbow-0.2.0"), None, "names match whole");
    }

    #[test]
    fn a_download_is_kept_only_when_its_checksum_matches() {
        let dir = std::env::temp_dir().join(format!("oxbow-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("setup.exe");
        let data = b"the installer";
        let good: String = Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect();

        let err = save_checked(&data[..], &dest, &"0".repeat(64)).unwrap_err();
        assert!(err.to_string().contains("checksum"), "{err}");
        assert!(!dest.exists(), "a bad download isn't kept");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nor its partial file");

        std::fs::write(&dest, b"the old one").unwrap();
        save_checked(&b"tampered"[..], &dest, &good).unwrap_err();
        assert_eq!(std::fs::read(&dest).unwrap(), b"the old one", "a bad download leaves what was there");

        save_checked(&data[..], &dest, &good).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_build_from_source_only_links_to_the_release() {
        // Tests run from target/, which has no installer next to it and isn't an AppImage.
        if std::env::var_os("APPIMAGE").is_none() {
            assert_eq!(Install::detect(), Install::Manual);
        }
        assert_eq!(Install::Manual.asset_name(v("0.2.0")), None);
        assert_eq!(Install::Installer.asset_name(v("0.2.0")).unwrap(), "oxbow-0.2.0-windows-x64-setup.exe");
        assert_eq!(Install::AppImage("/x".into()).asset_name(v("0.2.0")).unwrap(), "oxbow-0.2.0-x86_64.AppImage");
        assert!(!Install::AdminInstaller.installs_quietly(), "an all-users install asks first");
    }
}
