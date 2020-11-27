use crate::download::{
    append_to_path, download, download_with_sha256_file, move_if_exists,
    move_if_exists_with_sha256, write_file_create_dir, DownloadError,
};
use crate::mirror::{MirrorError, MirrorSection, RustupSection};
use crate::progress_bar::{progress_bar, ProgressBarMessage};
use console::style;
use reqwest::header::HeaderValue;
use scoped_threadpool::Pool;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{fs, io};

// Note: These platforms should match https://github.com/rust-lang/rustup.rs#other-installation-methods

/// Non-windows platforms
static PLATFORMS: &[&str] = &[
    "aarch64-linux-android",
    "aarch64-unknown-linux-gnu",
    "arm-linux-androideabi",
    "arm-unknown-linux-gnueabi",
    "arm-unknown-linux-gnueabihf",
    "armv7-linux-androideabi",
    "armv7-unknown-linux-gnueabihf",
    "i686-apple-darwin",
    "i686-linux-android",
    "i686-unknown-linux-gnu",
    "mips-unknown-linux-gnu",
    "mips64-unknown-linux-gnuabi64",
    "mips64el-unknown-linux-gnuabi64",
    "mipsel-unknown-linux-gnu",
    "powerpc-unknown-linux-gnu",
    "powerpc64-unknown-linux-gnu",
    "powerpc64le-unknown-linux-gnu",
    "s390x-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "x86_64-linux-android",
    "x86_64-unknown-freebsd",
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "x86_64-unknown-netbsd",
];

/// Windows platforms (platforms where rustup-init has a .exe extension)
static PLATFORMS_EXE: &[&str] = &[
    "i686-pc-windows-gnu",
    "i686-pc-windows-msvc",
    "x86_64-pc-windows-gnu",
    "x86_64-pc-windows-msvc",
];

quick_error! {
    #[derive(Debug)]
    pub enum SyncError {
        Io(err: io::Error) {
            from()
        }
        Download(err: DownloadError) {
            from()
        }
        Parse(err: toml::de::Error) {
            from()
        }
        Serialize(err: toml::ser::Error) {
            from()
        }
        StripPrefix(err: std::path::StripPrefixError) {
            from()
        }
        FailedDownloads(count: usize) {}
    }
}

pub fn get_platforms(target_platform: Option<&str>) -> Vec<String> {
    if let Some(platform) = target_platform {
        if PLATFORMS.contains(&platform) {
            vec![platform.into()]
        } else {
            vec![]
        }
    } else {
        // there's a lot of allocation going on here, we should fix this at
        // some point
        PLATFORMS.iter().cloned().map(|x| x.to_owned()).collect()
    }
}

pub fn get_platforms_exe(target_platform: Option<&str>) -> Vec<String> {
    if let Some(platform) = target_platform {
        if PLATFORMS_EXE.contains(&platform) {
            vec![platform.into()]
        } else {
            vec![]
        }
    } else {
        // there's a lot of allocation going on here, we should fix this at
        // some point
        PLATFORMS_EXE
            .iter()
            .cloned()
            .map(|x| x.to_owned())
            .collect()
    }
}

/// Synchronize one rustup-init file.
pub fn sync_one_init(
    path: &Path,
    source: &str,
    platform: &str,
    is_exe: bool,
    retries: usize,
    user_agent: &HeaderValue,
) -> Result<(), DownloadError> {
    let local_path = if is_exe {
        path.join("rustup/dist")
            .join(platform)
            .join("rustup-init.exe")
    } else {
        path.join("rustup/dist").join(platform).join("rustup-init")
    };

    let source_url = if is_exe {
        format!("{}/rustup/dist/{}/rustup-init.exe", source, platform)
    } else {
        format!("{}/rustup/dist/{}/rustup-init", source, platform)
    };

    download_with_sha256_file(&source_url, &local_path, retries, false, user_agent)?;

    Ok(())
}

/// Synchronize all rustup-init files.
pub fn sync_rustup_init(
    path: &Path,
    source: &str,
    platform: &Option<String>,
    prefix: String,
    threads: usize,
    retries: usize,
    user_agent: &HeaderValue,
) -> Result<(), SyncError> {
    let platforms = get_platforms(platform.as_deref());
    let platforms_exe = get_platforms_exe(platform.as_deref());

    let count = platforms.len() + platforms_exe.len();

    let (pb_thread, sender) = progress_bar(Some(count), prefix);

    let errors_occurred = AtomicUsize::new(0);

    Pool::new(threads as u32).scoped(|scoped| {
        let error_occurred = &errors_occurred;
        for platform in platforms {
            let s = sender.clone();
            scoped.execute(move || {
                if let Err(e) = sync_one_init(path, source, &platform, false, retries, user_agent) {
                    s.send(ProgressBarMessage::Println(format!(
                        "Downloading {} failed: {:?}",
                        path.display(),
                        e
                    )))
                    .expect("Channel send should not fail");
                    error_occurred.fetch_add(1, Ordering::Release);
                }
                s.send(ProgressBarMessage::Increment)
                    .expect("Channel send should not fail");
            })
        }

        for platform in platforms_exe {
            let s = sender.clone();
            scoped.execute(move || {
                if let Err(e) = sync_one_init(path, source, &platform, true, retries, user_agent) {
                    s.send(ProgressBarMessage::Println(format!(
                        "Downloading {} failed: {:?}",
                        path.display(),
                        e
                    )))
                    .expect("Channel send should not fail");
                    error_occurred.fetch_add(1, Ordering::Release);
                }
                s.send(ProgressBarMessage::Increment)
                    .expect("Channel send should not fail");
            })
        }
    });

    sender
        .send(ProgressBarMessage::Done)
        .expect("Channel send should not fail");
    pb_thread.join().expect("Thread join should not fail");

    let errors = errors_occurred.load(Ordering::Acquire);
    if errors == 0 {
        Ok(())
    } else {
        Err(SyncError::FailedDownloads(errors))
    }
}

#[derive(Deserialize, Debug)]
struct TargetUrls {
    url: String,
    hash: String,
    xz_url: String,
    xz_hash: String,
}

#[derive(Deserialize, Debug)]
struct Target {
    available: bool,

    #[serde(flatten)]
    target_urls: Option<TargetUrls>,
}

#[derive(Deserialize, Debug)]
struct Pkg {
    version: String,
    target: HashMap<String, Target>,
}

#[derive(Deserialize, Debug)]
struct Channel {
    #[serde(alias = "manifest-version")]
    manifest_version: String,
    date: String,
    pkg: HashMap<String, Pkg>,
}

/// Get the rustup file downloads, in pairs of URLs and sha256 hashes.
pub fn rustup_download_list(
    path: &Path,
    source: &str,
) -> Result<(String, Vec<(String, String)>), SyncError> {
    let channel_str = fs::read_to_string(path).map_err(DownloadError::Io)?;
    let channel: Channel = toml::from_str(&channel_str)?;

    Ok((
        channel.date,
        channel
            .pkg
            .into_iter()
            .flat_map(|(_, pkg)| {
                pkg.target
                    .into_iter()
                    .flat_map(|(_, target)| -> Vec<(String, String)> {
                        target
                            .target_urls
                            .map(|urls| vec![(urls.url, urls.hash), (urls.xz_url, urls.xz_hash)])
                            .into_iter()
                            .flatten()
                            .map(|(url, hash)| {
                                (
                                    url[source.len()..].trim_start_matches('/').to_string(),
                                    hash,
                                )
                            })
                            .collect()
                    })
            })
            .collect(),
    ))
}

pub fn sync_one_rustup_target(
    path: &Path,
    source: &str,
    url: &str,
    hash: &str,
    retries: usize,
    user_agent: &HeaderValue,
) -> Result<(), DownloadError> {
    // Chop off the source portion of the URL, to mimic the rest of the path
    //let target_url = path.join(url[source.len()..].trim_start_matches("/"));
    let target_url = format!("{}/{}", source, url);
    let target_path = path.join(url);

    download(
        &target_url,
        &target_path,
        Some(hash),
        retries,
        false,
        user_agent,
    )?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelHistoryFile {
    pub versions: HashMap<String, Vec<String>>,
}

pub fn latest_dates_from_channel_history(
    channel_history: &ChannelHistoryFile,
    versions: usize,
) -> Vec<String> {
    let mut dates: Vec<String> = channel_history
        .versions
        .keys()
        .map(|x| x.to_string())
        .collect();
    dates.sort();
    dates.reverse();
    dates.truncate(versions);
    dates
}

pub fn clean_old_files(
    path: &Path,
    keep_stables: Option<usize>,
    keep_betas: Option<usize>,
    keep_nightlies: Option<usize>,
    prefix: String,
) -> Result<(), SyncError> {
    // Handle all of stable/beta/nightly
    let mut files_to_keep: HashSet<String> = HashSet::new();
    if let Some(s) = keep_stables {
        let mut stable = get_channel_history(path, "stable")?;
        let latest_dates = latest_dates_from_channel_history(&stable, s);
        for date in latest_dates {
            if let Some(t) = stable.versions.get_mut(&date) {
                t.iter().for_each(|t| {
                    files_to_keep.insert(t.to_string());
                });
            }
        }
    }
    if let Some(b) = keep_betas {
        let mut beta = get_channel_history(path, "beta")?;
        let latest_dates = latest_dates_from_channel_history(&beta, b);
        for date in latest_dates {
            if let Some(t) = beta.versions.get_mut(&date) {
                //files_to_keep.append(&mut t);
                t.iter().for_each(|t| {
                    files_to_keep.insert(t.to_string());
                });
            }
        }
    }
    if let Some(n) = keep_nightlies {
        let mut nightly = get_channel_history(path, "nightly")?;
        let latest_dates = latest_dates_from_channel_history(&nightly, n);
        for date in latest_dates {
            if let Some(t) = nightly.versions.get_mut(&date) {
                t.iter().for_each(|t| {
                    files_to_keep.insert(t.to_string());
                });
            }
        }
    }

    let dist_path = path.join("dist");
    let mut files_to_delete: Vec<String> = vec![];

    for dir in fs::read_dir(dist_path)? {
        let dir = dir?.path();
        if dir.is_dir() {
            for full_path in fs::read_dir(dir)? {
                let full_path = full_path?.path();
                let file_path = full_path.strip_prefix(path)?;
                if let Some(file_path) = file_path.to_str() {
                    if !files_to_keep.contains(file_path) {
                        files_to_delete.push(file_path.to_string());
                    }
                }
            }
        }
    }

    // Progress bar!
    let (pb_thread, sender) = progress_bar(Some(files_to_delete.len()), prefix);

    for f in files_to_delete {
        if let Err(e) = fs::remove_file(path.join(&f)) {
            sender
                .send(ProgressBarMessage::Println(format!(
                    "Could not remove file {}: {:?}",
                    f, e
                )))
                .expect("Channel send should not fail");
        }
        sender
            .send(ProgressBarMessage::Increment)
            .expect("Channel send should not fail");
    }

    sender
        .send(ProgressBarMessage::Done)
        .expect("Channel send should not fail");
    pb_thread.join().expect("Thread join should not fail");

    Ok(())
}

pub fn get_channel_history(path: &Path, channel: &str) -> Result<ChannelHistoryFile, SyncError> {
    let channel_history_path = path.join(format!("mirror-{}-history.toml", channel));
    if channel_history_path.exists() {
        let ch_data = fs::read_to_string(channel_history_path)?;
        Ok(toml::from_str(&ch_data)?)
    } else {
        Ok(ChannelHistoryFile {
            versions: HashMap::new(),
        })
    }
}

pub fn add_to_channel_history(
    path: &Path,
    channel: &str,
    date: &str,
    files: &[(String, String)],
) -> Result<(), SyncError> {
    let mut channel_history = get_channel_history(path, channel)?;
    channel_history.versions.insert(
        date.to_string(),
        files.iter().map(|(f, _)| f.to_string()).collect(),
    );

    let ch_data = toml::to_string(&channel_history)?;

    let channel_history_path = path.join(format!("mirror-{}-history.toml", channel));
    write_file_create_dir(&channel_history_path, &ch_data)?;

    Ok(())
}

/// Synchronize a rustup channel (stable, beta, or nightly).
pub fn sync_rustup_channel(
    path: &Path,
    source: &str,
    threads: usize,
    target_platform: &Option<String>,
    target_extension: &Option<String>,
    prefix: String,
    channel: &str,
    retries: usize,
    user_agent: &HeaderValue,
) -> Result<(), SyncError> {
    // Download channel file
    let channel_url = format!("{}/dist/channel-rust-{}.toml", source, channel);
    let channel_path = path.join(format!("dist/channel-rust-{}.toml", channel));
    let channel_part_path = append_to_path(&channel_path, ".part");
    download_with_sha256_file(&channel_url, &channel_part_path, retries, true, user_agent)?;

    let release_url = format!("{}/rustup/release-{}.toml", source, channel);
    let release_path = path.join(format!("rustup/release-{}.toml", channel));
    let release_part_path = append_to_path(&release_path, ".part");

    // Download release file if stable
    if channel == "stable" {
        download(
            &release_url,
            &release_part_path,
            None,
            retries,
            false,
            user_agent,
        )?;
    }

    // Open toml file, find all files to download
    let (date, mut files) = rustup_download_list(&channel_part_path, source)?;

    if let Some(target_platform) = target_platform {
        // only sync the files from the target platform
        files = files
            .into_iter()
            .filter(|x| x.0.contains(target_platform))
            .collect();
    }

    if let Some(target_extension) = target_extension {
        // only sync the files that end in the target extension
        files = files
            .into_iter()
            .filter(|x| x.0.ends_with(target_extension))
            .collect();
    }

    // Create progress bar
    let (pb_thread, sender) = progress_bar(Some(files.len()), prefix);

    let errors_occurred = AtomicUsize::new(0);

    // Download files
    Pool::new(threads as u32).scoped(|scoped| {
        let error_occurred = &errors_occurred;
        for (url, hash) in &files {
            let s = sender.clone();
            scoped.execute(move || {
                if let Err(e) =
                    sync_one_rustup_target(&path, &source, &url, &hash, retries, user_agent)
                {
                    s.send(ProgressBarMessage::Println(format!(
                        "Downloading {} failed: {:?}",
                        path.display(),
                        e
                    )))
                    .expect("Channel send should not fail");
                    error_occurred.fetch_add(1, Ordering::Release);
                }
                s.send(ProgressBarMessage::Increment)
                    .expect("Channel send should not fail");
            })
        }
    });

    // Wait for progress bar to finish
    sender
        .send(ProgressBarMessage::Done)
        .expect("Channel send should not fail");
    pb_thread.join().expect("Thread join should not fail");

    let errors = errors_occurred.load(Ordering::Acquire);
    if errors == 0 {
        // Write channel history file
        add_to_channel_history(path, channel, &date, &files)?;
        move_if_exists_with_sha256(&channel_part_path, &channel_path)?;
        move_if_exists(&release_part_path, &release_path)?;
        Ok(())
    } else {
        Err(SyncError::FailedDownloads(errors))
    }
}

/// Synchronize rustup.
pub fn sync(
    path: &Path,
    mirror: &MirrorSection,
    rustup: &RustupSection,
    user_agent: &HeaderValue,
) -> Result<(), MirrorError> {
    eprintln!("{}", style("Syncing Rustup repositories...").bold());

    // Mirror rustup-init
    let prefix = format!("{} Syncing rustup-init files...", style("[1/5]").bold());
    if let Err(e) = sync_rustup_init(
        path,
        &rustup.source,
        &rustup.target_platform,
        prefix,
        rustup.download_threads,
        mirror.retries,
        user_agent,
    ) {
        eprintln!("Downloading rustup init files failed: {:?}", e);
        eprintln!("You will need to sync again to finish this download.");
    }

    let mut failures = false;

    // Mirror stable
    if rustup.keep_latest_stables != Some(0) {
        let prefix = format!("{} Syncing latest stable...    ", style("[2/5]").bold());
        if let Err(e) = sync_rustup_channel(
            path,
            &rustup.source,
            rustup.download_threads,
            &rustup.target_platform,
            &rustup.target_extension,
            prefix,
            "stable",
            mirror.retries,
            user_agent,
        ) {
            failures = true;
            eprintln!("Downloading stable release failed: {:?}", e);
            eprintln!("You will need to sync again to finish this download.");
        }
    } else {
        eprintln!("{} Skipping syncing stable.", style("[2/5]").bold());
    }

    // Mirror beta
    if rustup.keep_latest_betas != Some(0) {
        let prefix = format!("{} Syncing latest beta...      ", style("[3/5]").bold());
        if let Err(e) = sync_rustup_channel(
            path,
            &rustup.source,
            rustup.download_threads,
            &rustup.target_platform,
            &rustup.target_extension,
            prefix,
            "beta",
            mirror.retries,
            user_agent,
        ) {
            failures = true;
            eprintln!("Downloading beta release failed: {:?}", e);
            eprintln!("You will need to sync again to finish this download.");
        }
    } else {
        eprintln!("{} Skipping syncing beta.", style("[3/5]").bold());
    }

    // Mirror nightly
    if rustup.keep_latest_nightlies != Some(0) {
        let prefix = format!("{} Syncing latest nightly...   ", style("[4/5]").bold());
        if let Err(e) = sync_rustup_channel(
            path,
            &rustup.source,
            rustup.download_threads,
            &rustup.target_platform,
            &rustup.target_extension,
            prefix,
            "nightly",
            mirror.retries,
            user_agent,
        ) {
            failures = true;
            eprintln!("Downloading nightly release failed: {:?}", e);
            eprintln!("You will need to sync again to finish this download.");
        }
    } else {
        eprintln!("{} Skipping syncing nightly.", style("[4/5]").bold());
    }

    // If all succeeds, clean files
    if rustup.keep_latest_stables == None
        && rustup.keep_latest_betas == None
        && rustup.keep_latest_nightlies == None
    {
        eprintln!("{} Skipping cleaning files.", style("[5/5]").bold());
    } else if failures {
        eprintln!(
            "{} Skipping cleaning files due to download failures.",
            style("[5/5]").bold()
        );
    } else {
        let prefix = format!("{} Cleaning old files...       ", style("[5/5]").bold());
        if let Err(e) = clean_old_files(
            path,
            rustup.keep_latest_stables,
            rustup.keep_latest_betas,
            rustup.keep_latest_nightlies,
            prefix,
        ) {
            eprintln!("Cleaning old files failed: {:?}", e);
            eprintln!("You may need to sync again to clean these files.");
        }
    }

    eprintln!("{}", style("Syncing Rustup repositories complete!").bold());

    Ok(())
}
