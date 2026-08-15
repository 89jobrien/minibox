//! `mbx tui` — read-only terminal dashboard (container table + live event log).

pub async fn execute() -> anyhow::Result<()> {
    minibox_tui::run()
        .await
        .map_err(|e| anyhow::anyhow!("{e:?}"))
}
