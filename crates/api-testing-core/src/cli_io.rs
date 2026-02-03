use std::io::Read;
use std::path::PathBuf;

use anyhow::anyhow;

use crate::Result;

pub fn read_response_bytes(response: &str, stdin: &mut dyn Read) -> Result<Vec<u8>> {
    if response == "-" {
        let mut buf = Vec::new();
        stdin
            .read_to_end(&mut buf)
            .map_err(|_| anyhow!("error: failed to read response from stdin"))?;
        return Ok(buf);
    }

    let resp_path = PathBuf::from(response);
    if !resp_path.is_file() {
        return Err(anyhow!("Response file not found: {}", resp_path.display()));
    }

    std::fs::read(&resp_path).map_err(|_| {
        anyhow!(
            "error: failed to read response file: {}",
            resp_path.display()
        )
    })
}
