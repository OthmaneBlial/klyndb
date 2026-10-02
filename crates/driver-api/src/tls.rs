use crate::{Error, Result};
use tokio::io::AsyncReadExt;
use zeroize::Zeroizing;

async fn read_tls_file(path: &str, kind: &str) -> Result<Zeroizing<Vec<u8>>> {
    if !std::path::Path::new(path).is_absolute() || path.len() > 16384 || path.contains('\0') {
        return Err(Error::new(format!(
            "Choose an absolute {kind} certificate file path"
        )));
    }
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| Error::new(format!("Could not open the {kind} certificate file")))?;
    let metadata = file
        .metadata()
        .await
        .map_err(|_| Error::new(format!("Could not read the {kind} certificate file")))?;
    const MAX: u64 = 1024 * 1024;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX {
        return Err(Error::new(format!(
            "The {kind} file must be a nonempty regular file no larger than 1 MiB"
        )));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Error::new(format!("Could not read the {kind} certificate file")))?;
    if bytes.len() as u64 > MAX {
        return Err(Error::new(format!("The {kind} file exceeds 1 MiB")));
    }
    Ok(bytes)
}

/// Native PKCS#12 parsing validates the identity without exposing its key or parse details.
pub async fn load_client_identity(
    path: &str,
    password: Option<&str>,
) -> Result<(Zeroizing<Vec<u8>>, native_tls::Identity)> {
    let password = password.unwrap_or_default();
    validate_identity_password(password)?;
    let bytes = read_tls_file(path, "client identity").await?;
    let identity = native_tls::Identity::from_pkcs12(&bytes, password).map_err(|_| Error::new("Could not unlock the PKCS#12 client identity. Check the file and certificate password."))?;
    Ok((bytes, identity))
}

/// Both native connectors use the same bounded CA validation.
pub async fn load_ca_certificates(path: &str) -> Result<Vec<Vec<u8>>> {
    let bytes = read_tls_file(path, "CA").await?;
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

pub fn validate_identity_password(password: &str) -> Result<()> {
    if password.len() > 16384 || password.contains('\0') {
        return Err(Error::new("Invalid client certificate password"));
    }
    Ok(())
}
