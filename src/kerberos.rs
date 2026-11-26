//! Active Directory (Kerberos) sign-in for Windows integrated authentication.
//!
//! Inside the Flatpak the host's /etc is mounted at /run/host/etc, and its
//! krb5.conf points at files that do not exist in the sandbox, so
//! `prepare_config` flattens it into a private copy first.

use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};

/// The service principal SQL Server registers for itself:
/// `MSSQLSvc/<fqdn>:<port>` or, for a named instance, `MSSQLSvc/<fqdn>:<instance>`.
/// Short host names are expanded through DNS, as the Windows client does.
pub fn service_principal(host: &str, port: Option<u16>, instance: Option<&str>) -> String {
    let fqdn = canonical_host(host).unwrap_or_else(|| host.to_string());
    match (instance, port) {
        (_, Some(p)) => format!("MSSQLSvc/{fqdn}:{p}"),
        (Some(i), None) => format!("MSSQLSvc/{fqdn}:{i}"),
        (None, None) => format!("MSSQLSvc/{fqdn}:1433"),
    }
}

/// Resolves `host` to its canonical DNS name (getaddrinfo with AI_CANONNAME).
fn canonical_host(host: &str) -> Option<String> {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let c_host = CString::new(host).ok()?;
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_flags = libc::AI_CANONNAME;
    hints.ai_socktype = libc::SOCK_STREAM;
    let mut res: *mut libc::addrinfo = std::ptr::null_mut();
    let rc = unsafe { libc::getaddrinfo(c_host.as_ptr(), std::ptr::null(), &hints, &mut res) };
    if rc != 0 || res.is_null() {
        return None;
    }
    let name = unsafe {
        let p = (*res).ai_canonname;
        let n = (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned());
        libc::freeaddrinfo(res);
        n
    };
    name.filter(|n| !n.is_empty()).map(|n| n.to_lowercase())
}

/// Adds a hint to the cryptic GSSAPI errors people actually hit.
pub fn explain(err: &str) -> String {
    let lower = err.to_lowercase();
    let hint = if lower.contains("no kerberos credentials") || lower.contains("no credentials") {
        "You have no Kerberos ticket. Enter your domain account (user@DOMAIN.COM) and password, or run kinit on this computer."
    } else if lower.contains("server not found in kerberos database") {
        "The domain has no SPN for this server. Use the server's full DNS name, or set the SPN under Options (ask the DBA which one is registered: setspn -L <service account>)."
    } else if lower.contains("cannot find kdc") || lower.contains("cannot contact any kdc") {
        "No domain controller was found for your realm. Check /etc/krb5.conf (default_realm, dns_lookup_kdc) or join the domain with realm join."
    } else if lower.contains("clock skew") {
        "This computer's clock differs from the domain controller's by more than 5 minutes."
    } else if lower.contains("ticket expired") {
        "Your Kerberos ticket has expired. Enter your password to sign in again."
    } else {
        return err.to_string();
    };
    format!("{err}\n\n{hint}")
}

#[cfg(feature = "kerberos")]
mod imp {
    use libgssapi::credential::{Cred, CredUsage};
    use libgssapi::name::Name;
    use libgssapi::oid::{GSS_MECH_KRB5, GSS_NT_KRB5_PRINCIPAL, OidSet};

    fn mechs() -> Result<OidSet, String> {
        let mut s = OidSet::new();
        s.add(GSS_MECH_KRB5).map_err(|e| e.to_string())?;
        Ok(s)
    }

    /// The principal of the current Kerberos ticket, e.g. `jdoe@CORP.EXAMPLE.COM`.
    pub fn current_principal() -> Result<String, String> {
        let cred = Cred::acquire(None, None, CredUsage::Initiate, Some(&mechs()?)).map_err(|e| e.to_string())?;
        let name = cred.name().map_err(|e| e.to_string())?;
        Ok(name.to_string())
    }

    /// Gets a ticket for `principal` with `password` and stores it in the
    /// default credential cache (what kinit does).
    pub fn sign_in(principal: &str, password: &str) -> Result<String, String> {
        let name = Name::new(principal.as_bytes(), Some(GSS_NT_KRB5_PRINCIPAL)).map_err(|e| e.to_string())?;
        let cred = Cred::acquire_with_password(Some(&name), password, None, CredUsage::Initiate, Some(&mechs()?))
            .map_err(|e| e.to_string())?;
        cred.store(true, true, CredUsage::Initiate, Some(GSS_MECH_KRB5)).map_err(|e| e.to_string())?;
        Ok(cred.name().map(|n| n.to_string()).unwrap_or_else(|_| principal.to_string()))
    }
}

#[cfg(not(feature = "kerberos"))]
mod imp {
    const MISSING: &str = "This build of SQLCraft was made without Kerberos support (build with --features kerberos).";

    pub fn current_principal() -> Result<String, String> {
        Err(MISSING.into())
    }

    pub fn sign_in(_principal: &str, _password: &str) -> Result<String, String> {
        Err(MISSING.into())
    }
}

pub use imp::{current_principal, sign_in};

/// Accepts `user@REALM` as is and turns `DOMAIN\user` into `user@DOMAIN`.
pub fn principal_from_login(login: &str) -> String {
    let login = login.trim();
    match login.split_once('\\') {
        Some((domain, user)) => format!("{user}@{}", domain.to_uppercase()),
        None => login.to_string(),
    }
}

// ----- Flatpak sandbox configuration -----------------------------------------

const HOST_ROOT: &str = "/run/host";
const MAX_DEPTH: u32 = 8;

/// MIT krb5 only reads files in an includedir whose names are made of
/// letters, digits, dashes and underscores, or that end in `.conf`.
fn included_name(name: &str) -> bool {
    !name.starts_with('.')
        && (name.ends_with(".conf") || name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

/// Maps a host path to where the sandbox sees it, if it can see it at all.
fn visible(path: &str) -> Option<PathBuf> {
    let under_host = Path::new(HOST_ROOT).join(path.trim_start_matches('/'));
    if under_host.exists() {
        return Some(under_host);
    }
    let direct = PathBuf::from(path);
    direct.exists().then_some(direct)
}

/// Inlines every include/includedir so the result reads from the sandbox.
fn flatten(path: &Path, depth: u32, out: &mut String) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    for line in text.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("includedir ") {
            if depth < MAX_DEPTH {
                if let Some(dir) = visible(rest.trim()) {
                    let mut files: Vec<_> = std::fs::read_dir(&dir)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .filter(|e| included_name(&e.file_name().to_string_lossy()))
                        .map(|e| e.path())
                        .collect();
                    files.sort();
                    for f in files {
                        out.push('\n');
                        flatten(&f, depth + 1, out);
                    }
                }
            }
        } else if let Some(rest) = t.strip_prefix("include ") {
            if depth < MAX_DEPTH {
                if let Some(f) = visible(rest.trim()) {
                    out.push('\n');
                    flatten(&f, depth + 1, out);
                }
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
}

/// When running in the Flatpak, writes a sandbox-readable copy of the host's
/// krb5.conf and points KRB5_CONFIG at it. Must run before any other thread starts.
pub fn prepare_config() {
    if std::env::var_os("KRB5_CONFIG").is_some() || !Path::new("/.flatpak-info").exists() {
        return;
    }
    let host_conf = Path::new(HOST_ROOT).join("etc/krb5.conf");
    if !host_conf.exists() {
        return;
    }
    let mut text = String::new();
    flatten(&host_conf, 0, &mut text);
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let target = dir.join("sqlcraft-krb5.conf");
    if std::fs::write(&target, text).is_ok() {
        // SAFETY: called at the top of main, before any threads exist.
        unsafe { std::env::set_var("KRB5_CONFIG", &target) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principals() {
        assert_eq!(principal_from_login(r"corp\jdoe"), "jdoe@CORP");
        assert_eq!(principal_from_login("jdoe@CORP.EXAMPLE.COM"), "jdoe@CORP.EXAMPLE.COM");
    }

    #[test]
    fn spn_forms() {
        assert_eq!(service_principal("10.0.0.5", Some(1444), None), "MSSQLSvc/10.0.0.5:1444");
        assert_eq!(service_principal("10.0.0.5", None, Some("SQLEXPRESS")), "MSSQLSvc/10.0.0.5:SQLEXPRESS");
        assert_eq!(service_principal("10.0.0.5", None, None), "MSSQLSvc/10.0.0.5:1433");
    }

    #[test]
    fn include_names() {
        assert!(included_name("crypto-policies"));
        assert!(included_name("sssd.conf"));
        assert!(!included_name("backup~"));
        assert!(!included_name(".hidden"));
    }

    #[test]
    fn flattens_includes() {
        let dir = std::env::temp_dir().join(format!("sqlcraft-krb-test-{}", std::process::id()));
        let sub = dir.join("conf.d");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("a"), "[libdefaults]\n default_realm = EXAMPLE.COM\n").unwrap();
        std::fs::write(sub.join("skip~"), "bad\n").unwrap();
        let main = dir.join("krb5.conf");
        std::fs::write(&main, format!("includedir {}\ninclude /nonexistent/file\n[realms]\n", sub.display())).unwrap();
        let mut out = String::new();
        flatten(&main, 0, &mut out);
        assert!(out.contains("default_realm = EXAMPLE.COM"));
        assert!(!out.contains("bad"));
        assert!(!out.contains("include"));
        assert!(out.contains("[realms]"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
