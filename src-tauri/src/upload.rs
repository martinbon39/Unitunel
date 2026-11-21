use crate::config::Remote;
use std::path::Path;
use std::process::Command;

/// ssh/scp from the system (built into Windows 10+ and macOS), so the user's own keys and ~/.ssh/config apply.
fn ssh(program: &str) -> Command {
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "-o", "StrictHostKeyChecking=accept-new"]);
    c
}

fn run(mut c: Command) -> Result<(), String> {
    let out = c.output().map_err(|e| format!("ssh introuvable sur cet ordinateur ({e})"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err.trim().lines().last().unwrap_or("connexion refusée").to_string())
}

fn remote_dir(r: &Remote) -> Result<String, String> {
    let dir = r.dir.trim().trim_end_matches('/');
    if r.target.trim().is_empty() {
        return Err("Indique le serveur (utilisateur@adresse)".into());
    }
    if !dir.starts_with('/') {
        return Err("Le dossier sur le serveur doit être un chemin absolu (/home/…)".into());
    }
    Ok(dir.to_string())
}

fn mkdir(r: &Remote, dir: &str) -> Result<(), String> {
    let mut c = ssh("ssh");
    c.arg(r.target.trim()).arg(format!("mkdir -p -- '{}'", dir.replace('\'', "'\\''")));
    run(c)
}

/// Checks that the server answers and the folder can be created.
pub fn test(r: &Remote) -> Result<(), String> {
    mkdir(r, &remote_dir(r)?)
}

/// Copies the tunnel folder to the server and returns the text with its paths pointing there.
pub fn send(r: &Remote, local: &Path, text: &str) -> Result<String, String> {
    let dir = remote_dir(r)?;
    let name = local.file_name().and_then(|n| n.to_str()).ok_or("dossier du tunnel invalide")?;
    let remote = format!("{dir}/{name}");
    let l = local.display().to_string();
    let moved = text.replace(&format!("{l}{}", std::path::MAIN_SEPARATOR), &format!("{remote}/")).replace(&l, &remote);
    let md = local.join("tunnel.md");
    std::fs::write(&md, &moved).map_err(|e| e.to_string())?;
    let sent = mkdir(r, &dir).and_then(|()| {
        let mut c = ssh("scp");
        c.args(["-r", "-q"]).arg(local).arg(format!("{}:{dir}/", r.target.trim()));
        run(c)
    });
    if let Err(e) = sent {
        let _ = std::fs::write(&md, text);
        return Err(e);
    }
    if !r.keep_local {
        let _ = std::fs::remove_dir_all(local);
    }
    Ok(moved)
}
