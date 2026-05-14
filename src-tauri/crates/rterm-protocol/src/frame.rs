use anyhow::{bail, Context, Result};
use bytes::Bytes;
use quinn::{RecvStream, SendStream};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::config;

pub async fn write_frame(send: &mut SendStream, data: &[u8]) -> Result<()> {
    write_frame_io(send, data).await
}

pub async fn write_frame_io<W: AsyncWrite + Unpin>(send: &mut W, data: &[u8]) -> Result<()> {
    if data.len() > config::MAX_FRAME {
        bail!("frame too large");
    }
    send.write_all(&(data.len() as u32).to_be_bytes()).await?;
    send.write_all(data).await?;
    send.flush().await?;
    Ok(())
}

pub async fn read_frame(recv: &mut RecvStream) -> Result<Option<Bytes>> {
    read_frame_limited(recv, config::MAX_FRAME).await
}

pub async fn read_frame_limited(recv: &mut RecvStream, max_frame: usize) -> Result<Option<Bytes>> {
    let mut len = [0u8; 4];
    if !read_exact_io(recv, &mut len)
        .await
        .context("read frame length")?
    {
        return Ok(None);
    }

    read_body(recv, u32::from_be_bytes(len) as usize, max_frame).await
}

pub async fn read_frame_io<R: AsyncRead + Unpin>(recv: &mut R) -> Result<Option<Bytes>> {
    read_frame_io_limited(recv, config::MAX_FRAME).await
}

pub async fn read_frame_io_limited<R: AsyncRead + Unpin>(
    recv: &mut R,
    max_frame: usize,
) -> Result<Option<Bytes>> {
    let mut len = [0u8; 4];
    if !read_exact_io(recv, &mut len)
        .await
        .context("read frame length")?
    {
        return Ok(None);
    }

    read_body(recv, u32::from_be_bytes(len) as usize, max_frame).await
}

async fn read_body<R: AsyncRead + Unpin>(
    recv: &mut R,
    len: usize,
    max_frame: usize,
) -> Result<Option<Bytes>> {
    if len > max_frame {
        bail!("frame too large");
    }

    let mut data = vec![0u8; len];
    if !read_exact_io(recv, &mut data)
        .await
        .context("read frame body")?
    {
        bail!("unexpected EOF in frame body");
    }
    Ok(Some(Bytes::from(data)))
}

async fn read_exact_io<R: AsyncRead + Unpin>(recv: &mut R, mut out: &mut [u8]) -> Result<bool> {
    let mut read_any = false;
    while !out.is_empty() {
        let n = recv.read(out).await?;
        if n == 0 {
            if read_any {
                bail!("unexpected EOF");
            }
            return Ok(false);
        }
        read_any = true;
        let tmp = out;
        out = &mut tmp[n..];
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncWriteExt};

    #[tokio::test]
    async fn io_frames_round_trip() {
        let (mut writer, mut reader) = duplex(64);

        write_frame_io(&mut writer, b"hello").await.unwrap();
        writer.shutdown().await.unwrap();

        assert_eq!(
            read_frame_io(&mut reader).await.unwrap().unwrap(),
            b"hello"[..]
        );
        assert!(read_frame_io(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn empty_io_frame_round_trips() {
        let (mut writer, mut reader) = duplex(64);

        write_frame_io(&mut writer, b"").await.unwrap();
        writer.shutdown().await.unwrap();

        assert_eq!(read_frame_io(&mut reader).await.unwrap().unwrap(), b""[..]);
    }

    #[tokio::test]
    async fn oversized_io_write_is_rejected() {
        let mut output = Vec::new();
        let data = vec![0u8; config::MAX_FRAME + 1];

        assert!(write_frame_io(&mut output, &data).await.is_err());
    }

    #[tokio::test]
    async fn oversized_io_frame_is_rejected() {
        let mut encoded = Vec::new();
        encoded.extend_from_slice(&5u32.to_be_bytes());
        encoded.extend_from_slice(b"hello");
        let mut input = encoded.as_slice();

        assert!(read_frame_io_limited(&mut input, 4).await.is_err());
    }

    #[tokio::test]
    async fn partial_io_frame_length_is_rejected() {
        let mut input = &[0, 0][..];

        assert!(read_frame_io(&mut input).await.is_err());
    }

    #[tokio::test]
    async fn partial_io_frame_body_is_rejected() {
        let mut encoded = Vec::new();
        encoded.extend_from_slice(&5u32.to_be_bytes());
        encoded.extend_from_slice(b"he");
        let mut input = encoded.as_slice();

        assert!(read_frame_io(&mut input).await.is_err());
    }
}
