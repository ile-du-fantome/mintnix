pub mod error;
pub mod mod_info;

use std::{
    io::BufWriter,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use fs_err as fs;
use tracing::*;
use tracing_subscriber::fmt::format::FmtSpan;

#[derive(Debug)]
pub enum DRGInstallationType {
    Steam,
}

impl DRGInstallationType {
    pub fn from_exe_path() -> Result<Self> {
        let exe_name = std::env::current_exe()
            .context("could not determine running exe")?
            .file_name()
            .context("failed to get exe path")?
            .to_string_lossy()
            .to_lowercase();
        match exe_name.as_str() {
            "fsd-win64-shipping.exe" => Ok(Self::Steam),
            _ => bail!("unrecognized exe file name: {exe_name}"),
        }
    }
}

impl DRGInstallationType {
    pub fn from_pak_path<P: AsRef<Path>>(pak: P) -> Result<Self> {
        let pak_name = pak
            .as_ref()
            .file_name()
            .context("failed to get pak file name")?
            .to_string_lossy()
            .to_lowercase();
        match pak_name.as_str() {
            "fsd-windowsnoeditor.pak" => Ok(Self::Steam),
            _ => bail!("unrecognized pak file name: {pak_name}"),
        }
    }
    pub fn binaries_directory_name(&self) -> &'static str {
        "Win64"
    }
    pub fn main_pak_name(&self) -> &'static str {
        "FSD-WindowsNoEditor.pak"
    }
    pub fn hook_dll_name(&self) -> &'static str {
        "x3daudio1_7.dll"
    }
}

#[derive(Debug)]
pub struct DRGInstallation {
    pub root: PathBuf,
    pub installation_type: DRGInstallationType,
}

impl DRGInstallation {
    /// Returns the first Steam DRG installation found.
    pub fn find() -> Option<Self> {
        steamlocate::SteamDir::locate()
            .ok()
            .and_then(|steamdir| {
                steamdir
                    .find_app(548430)
                    .ok()
                    .flatten()
                    .map(|(app, library)| {
                        library
                            .resolve_app_dir(&app)
                            .join("FSD/Content/Paks/FSD-WindowsNoEditor.pak")
                    })
            })
            .and_then(|path| Self::from_pak_path(path).ok())
    }
    pub fn from_pak_path<P: AsRef<Path>>(pak: P) -> Result<Self> {
        let root = pak
            .as_ref()
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .context("failed to get pak parent directory")?
            .to_path_buf();
        Ok(Self {
            root,
            installation_type: DRGInstallationType::from_pak_path(pak)?,
        })
    }
    pub fn binaries_directory(&self) -> PathBuf {
        self.root
            .join("Binaries")
            .join(self.installation_type.binaries_directory_name())
    }
    pub fn paks_path(&self) -> PathBuf {
        self.root.join("Content").join("Paks")
    }
    pub fn main_pak(&self) -> PathBuf {
        self.root
            .join("Content")
            .join("Paks")
            .join(self.installation_type.main_pak_name())
    }
    pub fn modio_directory(&self) -> Option<PathBuf> {
        #[cfg(target_os = "linux")]
        {
            steamlocate::SteamDir::locate()
                .map(|s| {
                    s.path()
                        .join("steamapps/compatdata/548430/pfx/drive_c/users/Public/mod.io/2475")
                })
                .ok()
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }
}

pub fn setup_logging<P: AsRef<Path>>(
    log_path: P,
    target: &str,
) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    use tracing::metadata::LevelFilter;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{
        EnvFilter,
        field::RecordFields,
        filter,
        fmt::{
            self, FormatFields,
            format::{Pretty, Writer},
        },
    };

    /// Workaround for <https://github.com/tokio-rs/tracing/issues/1817>.
    struct NewType(Pretty);

    impl<'writer> FormatFields<'writer> for NewType {
        fn format_fields<R: RecordFields>(
            &self,
            writer: Writer<'writer>,
            fields: R,
        ) -> core::fmt::Result {
            self.0.format_fields(writer, fields)
        }
    }

    let f = fs::File::create(log_path.as_ref())?;
    let writer = BufWriter::new(f);
    let (log_file_appender, guard) = tracing_appender::non_blocking(writer);
    let debug_file_log = fmt::layer()
        .with_writer(log_file_appender)
        .fmt_fields(NewType(Pretty::default()))
        .with_ansi(false)
        .with_filter(filter::Targets::new().with_target(target, Level::DEBUG));
    let stderr_log = fmt::layer()
        .with_writer(std::io::stderr)
        .event_format(tracing_subscriber::fmt::format().without_time())
        .with_span_events(FmtSpan::CLOSE)
        .with_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        );
    let subscriber = tracing_subscriber::registry()
        .with(stderr_log)
        .with(debug_file_log);

    tracing::subscriber::set_global_default(subscriber)?;

    debug!("tracing subscriber setup");
    info!("writing logs to {:?}", log_path.as_ref().display());
    info!("version: {}", env!("CARGO_PKG_VERSION"));

    Ok(guard)
}
