use anyhow::{Context, Result};

pub fn copy_text(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().context("failed to access clipboard")?;
    clipboard
        .set_text(text.to_string())
        .context("failed to set clipboard text")?;
    Ok(())
}
