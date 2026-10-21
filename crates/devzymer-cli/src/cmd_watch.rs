use anyhow::Result;
use devzymer_core::paths;
use devzymer_core::store::Store;
use devzymer_graph::build_graph;
use devzymer_indexer::index::{IndexOptions, Indexer};
use notify::RecursiveMode;
use notify::Watcher;
use std::sync::mpsc;
use std::time::Duration;

use crate::Ctx;

struct Handler(mpsc::Sender<notify::Result<notify::Event>>);

impl notify::EventHandler for Handler {
    fn handle_event(&mut self, event: notify::Result<notify::Event>) {
        let _ = self.0.send(event);
    }
}

pub async fn run(ctx: &Ctx) -> Result<()> {
    let root = ctx.resolve_root();
    let db = paths::db_path(&root);
    if !db.exists() {
        anyhow::bail!(
            "índice não encontrado em {}. Execute `devzymer init` primeiro.",
            db.display()
        );
    }

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(Handler(tx))?;
    watcher.watch(&root, RecursiveMode::Recursive)?;

    println!(
        "observando {}. Pressione Ctrl+C para encerrar.",
        root.display()
    );

    let mut pending = false;
    let mut last_event = std::time::Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(event)) => {
                if is_interesting(&event) {
                    pending = true;
                    last_event = std::time::Instant::now();
                }
            }
            Ok(Err(err)) => {
                tracing::warn!(error = %err, "evento de filesystem inválido");
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if pending && last_event.elapsed() >= Duration::from_millis(800) {
            pending = false;
            let started = std::time::Instant::now();
            let store = Store::open(&db)?;
            let indexer = Indexer::new(&store, &root);
            let options = IndexOptions {
                root: root.clone(),
                scan: crate::cmd_status::scan_options(ctx),
                force_full: false,
                chunk_size: 256,
            };
            match indexer.index(&options) {
                Ok(report) => {
                    let _ = build_graph(&store)?;
                    println!(
                        "sincronizado: {} arquivo(s), {} símbolo(s) em {} ms",
                        report.files_indexed,
                        report.symbols,
                        started.elapsed().as_millis()
                    );
                }
                Err(err) => {
                    tracing::error!(error = %err, "falha ao sincronizar");
                    println!("erro ao sincronizar: {err}");
                }
            }
        }
    }
    Ok(())
}

fn is_interesting(event: &notify::Event) -> bool {
    use notify::EventKind;
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}
