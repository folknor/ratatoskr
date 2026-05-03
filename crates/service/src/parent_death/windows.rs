use std::io;

pub(super) fn configure_command(_command: &mut tokio::process::Command) -> io::Result<()> {
    Ok(())
}
