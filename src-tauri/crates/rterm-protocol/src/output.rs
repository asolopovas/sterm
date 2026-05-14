use anyhow::Result;
use std::io::Write;

pub fn write_public_pin(label: &str, pin: &str) -> Result<()> {
    let mut stderr = std::io::stderr().lock();
    write_public_pin_to(&mut stderr, label, pin)
}

fn write_public_pin_to(mut writer: impl Write, label: &str, pin: &str) -> Result<()> {
    writer.write_all(label.as_bytes())?;
    writer.write_all(b": ")?;
    writer.write_all(pin.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_pin_output_includes_label_and_pin() {
        let mut output = Vec::new();

        write_public_pin_to(&mut output, "label", "pin").unwrap();

        assert_eq!(output, b"label: pin\n");
    }
}
