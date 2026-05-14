use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use quinn::{RecvStream, SendStream};
use rterm_protocol::config;
use std::io::{Read, Write};
use tokio::sync::mpsc;

pub async fn run_pty(mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: config::DEFAULT_PTY_ROWS,
        cols: config::DEFAULT_PTY_COLS,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let shell = default_shell();
    let cmd = CommandBuilder::new(shell);
    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;

    let (pty_tx, mut pty_rx) = mpsc::channel::<Vec<u8>>(config::PTY_CHANNEL_CAPACITY);
    std::thread::spawn(move || {
        let mut buf = [0u8; config::IO_BUFFER_SIZE];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if pty_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        for data in input_rx {
            if writer.write_all(&data).is_err() {
                break;
            }
            let _ = writer.flush();
        }
    });

    let mut to_pty = tokio::spawn(async move {
        while let Some(data) = rterm_protocol::read_frame(&mut recv).await? {
            if input_tx.send(data.to_vec()).is_err() {
                break;
            }
        }
        Ok::<_, anyhow::Error>(())
    });

    let mut from_pty = tokio::spawn(async move {
        while let Some(data) = pty_rx.recv().await {
            rterm_protocol::write_frame(&mut send, &data).await?;
        }
        let _ = send.finish();
        Ok::<_, anyhow::Error>(())
    });

    tokio::select! {
        result = &mut to_pty => {
            result??;
            from_pty.await??;
        }
        result = &mut from_pty => {
            to_pty.abort();
            result??;
        }
    }

    let _ = child.kill();
    Ok(())
}

fn default_shell() -> String {
    #[cfg(windows)]
    {
        std::env::var(config::DEFAULT_WINDOWS_SHELL_ENV)
            .unwrap_or_else(|_| config::DEFAULT_WINDOWS_SHELL.to_string())
    }
    #[cfg(not(windows))]
    {
        std::env::var(config::DEFAULT_UNIX_SHELL_ENV)
            .unwrap_or_else(|_| config::DEFAULT_UNIX_SHELL.to_string())
    }
}
