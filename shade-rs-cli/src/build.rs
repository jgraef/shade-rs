use std::{
    path::PathBuf,
    time::Duration,
};

use shade_rs_build::{
    ui::compile_ui,
    util::watch::WatchFiles,
};

use crate::{
    util::shutdown::GracefulShutdown,
    Error,
};

/// Build assets and UI.
#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(flatten)]
    build_options: BuildOptions,
}

impl Args {
    pub async fn run(self) -> Result<(), Error> {
        let mut shutdown = GracefulShutdown::new();

        self.build_options.spawn(&mut shutdown).await?;

        shutdown.join().await
    }
}

#[derive(Debug, clap::Args)]
pub struct BuildOptions {
    /// Path to the dist directory. This is where the generated files will be
    /// stored.
    #[arg(long = "dist", env = "DIST", default_value = "./dist/")]
    pub dist_path: PathBuf,

    /// Path to the UI crate.
    #[arg(long, env = "UI", default_value = "./shade-rs-ui/")]
    pub ui_path: PathBuf,

    /// Watch for file changes.
    #[arg(long)]
    pub watch: bool,

    /// After a file change, wait N seconds before rebuilding to avoid to many
    /// rebuild events.
    #[arg(long, default_value = "2")]
    pub debounce: f32,

    /// Disable debounce.
    #[arg(long)]
    pub no_debounce: bool,

    #[arg(long)]
    pub release: bool,

    /// Start with a clean build.
    #[arg(long)]
    pub clean: bool,
}

impl BuildOptions {
    pub async fn spawn(&self, shutdown: &mut GracefulShutdown) -> Result<(), Error> {
        let debounce = (!self.no_debounce).then(|| Duration::from_secs_f32(self.debounce));

        let dist_ui = self.dist_path.join("ui");
        let clean = self.clean || self.release;
        compile_ui(&self.ui_path, &dist_ui, clean, self.release).await?;

        if self.watch {
            tracing::info!("Watching for file changes...");

            let ui_path = self.ui_path.clone();
            let mut watch_files = WatchFiles::new()?;
            watch_files.watch(&ui_path)?;

            let token = shutdown.token();
            let release = self.release;
            shutdown.spawn(async move {
                loop {
                    tokio::select! {
                        _ = token.cancelled() => break,
                        changes_option = watch_files.next(debounce) => {
                            let Some(_changes) = changes_option else { break; };
                            if let Err(error) = compile_ui(&ui_path, &dist_ui, false, release).await {
                                tracing::error!(%error);
                            }
                        }
                    }
                }

                Ok(())
            });
        }

        Ok(())
    }
}
