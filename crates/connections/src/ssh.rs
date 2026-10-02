use klyndb_driver_api::{Error, Result};

pub const OPTIONS: &[&str] = &[
    "ssh_host",
    "ssh_port",
    "ssh_user",
    "ssh_auth",
    "ssh_identity",
    "ssh_fingerprint",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: String,
    pub identity: Option<String>,
    pub fingerprint: String,
}

pub fn parse(url: &url::Url) -> Result<Option<Config>> {
    let mut options = std::collections::HashMap::new();
    for (k, v) in url.query_pairs().filter(|(k, _)| k.starts_with("ssh_")) {
        if !OPTIONS.contains(&k.as_ref())
            || options.insert(k.into_owned(), v.into_owned()).is_some()
        {
            return Err(Error::new("Unsupported or repeated SSH option"));
        }
    }
    if options.is_empty() {
        return Ok(None);
    }
    if url
        .query_pairs()
        .any(|(k, _)| ["host", "hostaddr", "port"].contains(&k.as_ref()))
    {
        return Err(Error::new(
            "For SSH, put the database hostname and port in the URL authority, without routing query options",
        ));
    }
    let required = |key| {
        options
            .get(key)
            .filter(|v| !v.is_empty())
            .cloned()
            .ok_or_else(|| Error::new("Enter the SSH host, user and verified SHA256 fingerprint"))
    };
    let host = required("ssh_host")?;
    if host.len() > 253
        || host.contains(['/', '\\', '@', ':', '[', ']'])
            && host.parse::<std::net::IpAddr>().is_err()
        || host.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(Error::new(
            "Enter an SSH hostname or IP address without a port",
        ));
    }
    let host = match host.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.to_string(),
        Err(_) => url::Host::parse(&host)
            .map_err(|_| Error::new("Invalid SSH hostname"))?
            .to_string(),
    };
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let user = required("ssh_user")?;
    if user.len() > 128 || user.chars().any(char::is_control) {
        return Err(Error::new("Invalid SSH username"));
    }
    let port = options.get("ssh_port").map_or(Ok(22), |s| {
        s.parse::<u16>()
            .map_err(|_| Error::new("SSH port must be 1–65535"))
    })?;
    if port == 0 {
        return Err(Error::new("SSH port must be 1–65535"));
    }
    let fingerprint = required("ssh_fingerprint")?;
    if !fingerprint.starts_with("SHA256:")
        || fingerprint.len() != 50
        || !fingerprint[7..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    {
        return Err(Error::new(
            "Use the verified SSH host key fingerprint in SHA256:… format",
        ));
    }
    let auth = options
        .get("ssh_auth")
        .cloned()
        .unwrap_or_else(|| "agent".into());
    if !["agent", "key", "password"].contains(&auth.as_str()) {
        return Err(Error::new(
            "Choose SSH agent, private key or password authentication",
        ));
    }
    let identity = options.get("ssh_identity").cloned();
    if auth == "key"
        && identity.as_ref().is_none_or(|p| {
            !std::path::Path::new(p).is_absolute() || p.len() > 16384 || p.contains('\0')
        })
    {
        return Err(Error::new("Choose an absolute SSH private key file path"));
    }
    if auth != "key" && identity.is_some() {
        return Err(Error::new(
            "SSH identity files require private-key authentication",
        ));
    }
    Ok(Some(Config {
        host,
        port,
        user,
        auth,
        identity,
        fingerprint,
    }))
}

pub fn credential_key(id: &str) -> String {
    format!("ssh-{id}")
}
pub fn validate_password(password: &str) -> Result<()> {
    if password.len() > 16384 || password.contains('\0') {
        return Err(Error::new("Invalid SSH password or key passphrase"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_options_enforce_host_trust_and_separate_secrets() {
        let base = format!(
            "postgresql://alice@db.internal/db?ssh_host=bastion.example&ssh_user=alice&ssh_fingerprint=SHA256:{}",
            "A".repeat(43)
        );
        let config = parse(&url::Url::parse(&base).unwrap()).unwrap().unwrap();
        assert_eq!((config.port, config.auth.as_str()), (22, "agent"));
        for suffix in [
            "&ssh_host=other",
            "&hostaddr=127.0.0.1",
            "&host=other",
            "&port=5433",
            "&ssh_password=secret",
            "&ssh_port=0",
            "&ssh_port=65536",
            "&ssh_auth=unknown",
            "&ssh_identity=relative.key",
            "&ssh_user=%00",
        ] {
            assert!(
                parse(&url::Url::parse(&format!("{base}{suffix}")).unwrap()).is_err(),
                "{suffix}"
            );
        }
        for (key, value) in [
            ("ssh_host", "host:22"),
            ("ssh_host", "bad host"),
            ("ssh_fingerprint", "SHA256:short"),
            ("ssh_user", ""),
        ] {
            let mut url = url::Url::parse(&base).unwrap();
            let values: Vec<_> = url
                .query_pairs()
                .filter(|(k, _)| k != key)
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            url.query_pairs_mut()
                .clear()
                .extend_pairs(values)
                .append_pair(key, value);
            assert!(parse(&url).is_err());
        }
        let mut key = url::Url::parse(&base).unwrap();
        key.query_pairs_mut()
            .append_pair("ssh_auth", "key")
            .append_pair(
                "ssh_identity",
                &std::env::temp_dir().join("private key").to_string_lossy(),
            );
        assert!(parse(&key).unwrap().unwrap().identity.is_some());
        assert!(validate_password("pass\0word").is_err());
        assert!(validate_password(&"x".repeat(16385)).is_err());
        assert_ne!(credential_key("id"), crate::client_identity_key("id"));
        assert_ne!(credential_key("id"), "id");
    }
}
