use crate::{Error, Result};
use tokio::io::AsyncReadExt;

/// Read a bounded public CA file and validate every certificate before using it.
/// Returning DER lets both native driver connectors use the same validation.
pub async fn load_ca_certificates(path: &str) -> Result<Vec<Vec<u8>>> {
    if !std::path::Path::new(path).is_absolute() || path.len() > 16384 || path.contains('\0') {
        return Err(Error::new("Choose an absolute CA certificate file path"));
    }
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| Error::new("Could not open the CA certificate file"))?;
    let metadata = file
        .metadata()
        .await
        .map_err(|_| Error::new("Could not read the CA certificate file"))?;
    const MAX: u64 = 1024 * 1024;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX {
        return Err(Error::new(
            "The CA file must be a nonempty regular file no larger than 1 MiB",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Error::new("Could not read the CA certificate file"))?;
    if bytes.len() as u64 > MAX {
        return Err(Error::new("The CA file exceeds 1 MiB"));
    }
    let certs = native_tls::Certificate::from_der(&bytes)
        .map(|c| vec![c])
        .or_else(|_| native_tls::Certificate::stack_from_pem(&bytes))
        .map_err(|_| Error::new("The CA file must contain valid PEM or DER certificates"))?;
    if certs.is_empty() || certs.len() > 512 {
        return Err(Error::new("The CA file must contain 1–512 certificates"));
    }
    certs
        .into_iter()
        .map(|c| {
            c.to_der()
                .map_err(|_| Error::new("Could not decode the CA certificate"))
        })
        .collect()
}
