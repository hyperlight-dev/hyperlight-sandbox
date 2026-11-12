fn main() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        hyperlight_sandbox_worker::serve()
    }
    #[cfg(not(unix))]
    {
        anyhow::bail!("the experimental SDK process worker currently requires Unix")
    }
}
