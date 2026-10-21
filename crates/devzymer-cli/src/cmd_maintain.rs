//! Manutenção do índice: checkpoint, espaço livre e cache.

use anyhow::{bail, Result};
use devzymer_core::paths;
use devzymer_core::store::Store;

use crate::Ctx;

/// Mantém o índice enxuto e reporta o que foi feito.
///
/// A operação é reversível: nada é apagado do índice, apenas o espaço livre
/// é devolvido e o cache de recuperação é limpo.
pub fn run(ctx: &Ctx, vacuum: bool) -> Result<()> {
    let root = ctx.resolve_root();
    let db = paths::db_path(&root);
    if !db.exists() {
        bail!(
            "indice nao encontrado em {}. Execute `devzymer init` primeiro.",
            db.display()
        );
    }
    let store = Store::open(&db)?;

    let before = store.storage_metrics()?;
    let wal = db.with_extension("sqlite3-wal");
    let wal_before = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);

    let checkpoint = store.checkpoint()?;
    let freed = if vacuum {
        store.incremental_vacuum(4096)?
    } else {
        0
    };
    store.cache_clear()?;

    let after = store.storage_metrics()?;
    let wal_after = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);

    let payload = serde_json::json!({
        "index": db.display().to_string(),
        "pages": after.page_count,
        "page_size": after.page_size,
        "free_pages_before": before.freelist_pages,
        "free_pages_after": after.freelist_pages,
        "bytes_before": before.total_bytes(),
        "bytes_after": after.total_bytes(),
        "wal_bytes_before": wal_before,
        "wal_bytes_after": wal_after,
        "checkpoint_completed": checkpoint.completed(),
        "vacuum_freed_pages": freed,
    });

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("manutencao do indice: {}", db.display());
    println!(
        "  tamanho: {:.1} MB -> {:.1} MB",
        before.total_bytes() as f64 / 1_048_576.0,
        after.total_bytes() as f64 / 1_048_576.0
    );
    println!(
        "  paginas livres: {} -> {}",
        before.freelist_pages, after.freelist_pages
    );
    println!(
        "  log de escrita: {:.1} MB -> {:.1} MB",
        wal_before as f64 / 1_048_576.0,
        wal_after as f64 / 1_048_576.0
    );
    if checkpoint.completed() {
        println!("  checkpoint: concluido");
    } else {
        println!("  checkpoint: nao concluido, o log foi mantido como esta");
    }
    if vacuum {
        println!("  paginas devolvidas: {}", freed);
    } else {
        println!("  use `devzymer maintain --vacuum` para devolver o espaco livre");
    }
    Ok(())
}
