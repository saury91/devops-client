use rcgen::{BasicConstraints, CertificateParams, DnType, DistinguishedName, IsCa, KeyPair};
use std::path::PathBuf;
use time::{Duration, OffsetDateTime};

use crate::config::get_settings_dir;

pub struct CertPaths {
    pub cert_pem: PathBuf,
    pub key_pem: PathBuf,
}

fn get_cert_dir() -> PathBuf {
    get_settings_dir().join("certs")
}

pub fn generate_cert() -> Result<CertPaths, String> {
    let cert_dir = get_cert_dir();
    std::fs::create_dir_all(&cert_dir).map_err(|e| e.to_string())?;

    let cert_path = cert_dir.join("localhost.pem");
    let key_path = cert_dir.join("localhost-key.pem");

    if cert_path.exists() && key_path.exists() {
        return Ok(CertPaths {
            cert_pem: cert_path,
            key_pem: key_path,
        });
    }

    let ca_key = KeyPair::generate().map_err(|e| e.to_string())?;
    let mut ca_params = CertificateParams::default();
    let mut ca_dn = DistinguishedName::new();
    ca_dn.push(DnType::CommonName, "DevOps Client CA");
    ca_params.distinguished_name = ca_dn;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let now = OffsetDateTime::now_utc();
    ca_params.not_before = now;
    ca_params.not_after = now + Duration::days(365 * 10);

    let ca_cert = ca_params.self_signed(&ca_key).map_err(|e| e.to_string())?;
    let ca_cert_pem = ca_cert.pem();

    let ca_path = cert_dir.join("ca.pem");
    std::fs::write(&ca_path, &ca_cert_pem).map_err(|e| e.to_string())?;

    let server_key = KeyPair::generate().map_err(|e| e.to_string())?;
    let mut server_params =
        CertificateParams::new(vec!["127.0.0.1".to_string(), "localhost".to_string()])
            .map_err(|e| e.to_string())?;
    let mut server_dn = DistinguishedName::new();
    server_dn.push(DnType::CommonName, "localhost");
    server_params.distinguished_name = server_dn;
    server_params.not_before = now;
    server_params.not_after = now + Duration::days(365 * 2);

    let server_cert = server_params
        .signed_by(&server_key, &ca_cert, &ca_key)
        .map_err(|e| e.to_string())?;

    let server_cert_pem = server_cert.pem();

    std::fs::write(&cert_path, &server_cert_pem).map_err(|e| e.to_string())?;
    std::fs::write(&key_path, server_key.serialize_pem()).map_err(|e| e.to_string())?;

    install_ca_cert(&ca_path);

    Ok(CertPaths {
        cert_pem: cert_path,
        key_pem: key_path,
    })
}

fn install_ca_cert(ca_path: &PathBuf) {
    let path_str = ca_path.to_string_lossy().to_string();

    if cfg!(target_os = "macos") {
        std::process::Command::new("security")
            .args([
                "add-trusted-cert",
                "-d",
                "-r",
                "trustRoot",
                "-p",
                "ssl",
                "-k",
                "/Library/Keychains/System.keychain",
                &path_str,
            ])
            .output()
            .ok();
    } else if cfg!(target_os = "linux") {
        let dest = "/usr/local/share/ca-certificates/devops-client-ca.crt";
        std::fs::copy(ca_path, dest).ok();
        std::process::Command::new("update-ca-certificates")
            .output()
            .ok();
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("certutil")
            .args(["-addstore", "-f", "ROOT", &path_str])
            .output()
            .ok();
    }
}
