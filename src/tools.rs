//! Les outils MCP du morph : distributions, commandes, fichiers, chemins.

use crate::mcp::{str_prop, tool};
use crate::wsl::{self, decode, exec_args};
use serde_json::{json, Value};
use std::time::Duration;

const DELAI_DEFAUT_S: u64 = 120;
const DELAI_MAX_S: u64 = 1800;
/// Le démarrage d'une distribution arrêtée s'ajoute au délai de la commande.
const MARGE_DEMARRAGE: Duration = Duration::from_secs(45);
const TETE: usize = 4_000;
const QUEUE: usize = 8_000;
const LECTURE_MAX: usize = 100_000;
const ENTREES_MAX: usize = 500;

/// Commandes qui détruisent, arrêtent ou changent la sécurité : elles
/// attendent l'accord de l'utilisateur (`confirmed=true`).
const COMMANDES_A_RISQUE: &[&str] = &[
    "rm ",
    "rm\t",
    "rmdir",
    "shred",
    "mkfs",
    "dd ",
    "wipefs",
    "fdisk",
    "parted",
    "shutdown",
    "reboot",
    "poweroff",
    "halt",
    "init 0",
    "kill ",
    "pkill",
    "killall",
    "chmod -r",
    "chown -r",
    "chmod 777",
    "git push --force",
    "git push -f",
    "git reset --hard",
    "git clean",
    "truncate ",
    "> /dev/sd",
    "apt remove",
    "apt purge",
    "apt-get remove",
    "apt-get purge",
    "apt autoremove",
    "dnf remove",
    "pacman -r",
    "docker rm",
    "docker rmi",
    "docker system prune",
    "docker volume rm",
    "drop database",
    "drop table",
    "userdel",
    "passwd",
    "visudo",
    "crontab -r",
    "iptables",
    "ufw ",
    "mv / ",
    ":(){",
    "wsl.exe --unregister",
    "wsl --unregister",
];

pub fn tools_list() -> Value {
    let distro = || str_prop("Distribution (wsl_distros). Omise : celle par défaut.");
    let confirmed = json!({ "type": "boolean", "description": "true seulement après accord explicite de l'utilisateur" });
    json!({ "tools": [
        tool(
            "wsl_distros",
            "Liste les distributions WSL installées (nom, état, version, celle par défaut).",
            json!({}),
            &[],
        ),
        tool(
            "wsl_run",
            "Exécute une commande bash dans une distribution WSL et rend sa sortie (code, stdout, stderr). Non interactive : utilisez -y, --no-pager… Commande destructrice ou as_root : confirmation demandée.",
            json!({
                "command": str_prop("Commande bash"),
                "distro": distro(),
                "cwd": str_prop("Dossier de travail : chemin Linux (/home/…, ~/projet) ou Windows (C:\\…). Défaut : ~"),
                "timeout_s": { "type": "integer", "description": "Délai en secondes (défaut 120, max 1800)" },
                "as_root": { "type": "boolean", "description": "Exécuter en root (sans mot de passe sudo). Toujours soumis à confirmation." },
                "confirmed": confirmed.clone(),
            }),
            &["command"],
        ),
        tool(
            "wsl_read_file",
            "Lit un fichier texte d'une distribution WSL (chemin Linux, relatif à ~ s'il ne commence pas par /).",
            json!({
                "path": str_prop("Chemin Linux du fichier"),
                "distro": distro(),
                "max_chars": { "type": "integer", "description": "Taille lue au plus (défaut 100000)" },
            }),
            &["path"],
        ),
        tool(
            "wsl_write_file",
            "Écrit un fichier texte dans une distribution WSL (crée les dossiers manquants, remplace le fichier sauf avec append).",
            json!({
                "path": str_prop("Chemin Linux du fichier"),
                "content": str_prop("Contenu complet du fichier"),
                "distro": distro(),
                "append": { "type": "boolean", "description": "Ajouter à la fin au lieu de remplacer" },
            }),
            &["path", "content"],
        ),
        tool(
            "wsl_list_dir",
            "Liste un dossier d'une distribution WSL (type, taille, chemin). .git, node_modules, target… sont cités sans être parcourus.",
            json!({
                "path": str_prop("Chemin Linux du dossier. Défaut : ~"),
                "distro": distro(),
                "depth": { "type": "integer", "description": "Profondeur (1 à 4, défaut 1)" },
            }),
            &[],
        ),
        tool(
            "wsl_path",
            "Convertit un chemin entre Windows et WSL : C:\\Users\\… → /mnt/c/Users/…, et /home/… → \\\\wsl.localhost\\Distro\\home\\… (ouvrable depuis Windows).",
            json!({ "path": str_prop("Chemin Windows ou Linux"), "distro": distro() }),
            &["path"],
        ),
    ]})
}

pub async fn call(name: &str, args: Value) -> Result<Value, String> {
    match name {
        "wsl_distros" => distros().await,
        "wsl_run" => run(&args).await,
        "wsl_read_file" => read_file(&args).await,
        "wsl_write_file" => write_file(&args).await,
        "wsl_list_dir" => list_dir(&args).await,
        "wsl_path" => path(&args).await,
        autre => Err(format!("outil inconnu : {autre}")),
    }
}

fn texte<'a>(args: &'a Value, cle: &str) -> Option<&'a str> {
    args.get(cle).and_then(Value::as_str)
}

fn requis<'a>(args: &'a Value, cle: &str) -> Result<&'a str, String> {
    texte(args, cle)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("paramètre « {cle} » manquant"))
}

async fn distros() -> Result<Value, String> {
    let liste = wsl::distributions().await?;
    Ok(json!({ "distros": liste }))
}

/// La commande touche-t-elle à quelque chose de difficile à défaire ?
pub fn risque(commande: &str) -> Option<&'static str> {
    let bas = format!(" {} ", commande.to_lowercase());
    let bas = bas.replace(['\n', ';', '&', '|', '(', ')', '`'], " ");
    COMMANDES_A_RISQUE.iter().copied().find(|motif| {
        let m = motif.trim_end();
        // En début de mot : « rm » sonne dans « sudo rm -rf », pas dans « term ».
        bas.match_indices(m).any(|(i, _)| {
            let avant = bas[..i].chars().last().unwrap_or(' ');
            let apres_ok = !motif.ends_with(' ') || bas[i + m.len()..].starts_with([' ', '\t']);
            (avant.is_whitespace() || avant == '/' || !avant.is_alphanumeric()) && apres_ok
        })
    })
}

/// Garde la tête et la fin d'une sortie longue : la fin d'une compilation
/// porte l'erreur, le début dit ce qui a été lancé.
pub fn tronque(s: &str) -> String {
    let t = s.trim_end();
    let n = t.chars().count();
    if n <= TETE + QUEUE {
        return t.to_string();
    }
    let tete: String = t.chars().take(TETE).collect();
    let queue: String = t.chars().skip(n - QUEUE).collect();
    format!(
        "{tete}\n… ({} caractères omis) …\n{queue}",
        n - TETE - QUEUE
    )
}

async fn run(args: &Value) -> Result<Value, String> {
    let commande = requis(args, "command")?;
    let root = args["as_root"] == json!(true);
    if args["confirmed"] != json!(true) {
        let motif = risque(commande);
        if root || motif.is_some() {
            return Ok(json!({
                "needs_confirmation": true,
                "command": commande,
                "as_root": root,
                "reason": motif.map_or_else(
                    || "exécution en root".to_string(),
                    |m| format!("commande sensible (« {} »)", m.trim())
                ),
                "next": "Commande non exécutée. Décrivez-la à l'utilisateur et demandez son accord ; s'il accepte, rappelez l'outil avec confirmed=true."
            }));
        }
    }
    let distro = wsl::choisir(texte(args, "distro")).await?;
    let delai = args["timeout_s"]
        .as_u64()
        .unwrap_or(DELAI_DEFAUT_S)
        .clamp(1, DELAI_MAX_S);
    let secondes = delai.to_string();
    // bash de connexion pour retrouver le PATH de l'utilisateur (nvm, cargo,
    // pyenv…) ; sh sur les distributions sans bash (Alpine).
    let lanceur =
        r#"if command -v bash >/dev/null 2>&1; then exec bash -lc "$1"; else exec sh -lc "$1"; fi"#;
    let a = exec_args(
        &distro,
        texte(args, "cwd"),
        root,
        &[
            "timeout", "-k", "5", &secondes, "sh", "-c", lanceur, "locaryn", commande,
        ],
    );
    let s = wsl::wsl(&a, None, Duration::from_secs(delai) + MARGE_DEMARRAGE).await?;
    let (stderr, avis) = separer_avis_wsl(&decode(&s.stderr));
    let mut r = json!({
        "distro": distro,
        "exit_code": s.code,
        "stdout": tronque(&decode(&s.stdout)),
        "stderr": tronque(&stderr),
    });
    if !avis.is_empty() {
        r["wsl_notice"] = json!(avis.join(
            "
"
        ));
    }
    if s.code == Some(124) {
        r["timed_out"] = json!(true);
        r["note"] = json!(format!(
            "Commande interrompue après {delai} s. Pour un serveur ou une tâche longue : `setsid nohup … > ~/journal.log 2>&1 &` puis lisez le journal."
        ));
    }
    if stderr.contains("a terminal is required") || stderr.contains("a password is required") {
        r["note"] = json!("sudo demande un mot de passe, impossible à saisir ici. Relancez sans sudo avec as_root=true (après accord de l'utilisateur).");
    }
    Ok(r)
}

/// WSL glisse ses propres avis dans la sortie d'erreur (« wsl: Failed to
/// start the systemd user session… ») : ils ne viennent pas de la commande,
/// et le modèle les prenait pour un échec.
pub fn separer_avis_wsl(stderr: &str) -> (String, Vec<String>) {
    let mut avis = Vec::new();
    let mut reste = Vec::new();
    for ligne in stderr.lines() {
        if ligne.starts_with("wsl: ") {
            avis.push(ligne.trim_start_matches("wsl: ").to_string());
        } else {
            reste.push(ligne);
        }
    }
    (
        reste.join(
            "
",
        ),
        avis,
    )
}

async fn read_file(args: &Value) -> Result<Value, String> {
    let chemin = requis(args, "path")?;
    let distro = wsl::choisir(texte(args, "distro")).await?;
    let max = args["max_chars"]
        .as_u64()
        .map_or(LECTURE_MAX, |m| m.clamp(1, 1_000_000) as usize);
    // Quatre octets par caractère au plus : on lit large, on coupe en Rust.
    let octets_max = (max * 4 + 1).to_string();
    let a = exec_args(
        &distro,
        None,
        false,
        &["head", "-c", &octets_max, "--", chemin],
    );
    let s = wsl::wsl(&a, None, Duration::from_secs(60)).await?;
    if s.code != Some(0) {
        return Err(format!("Lecture impossible : {}", decode(&s.stderr).trim()));
    }
    if s.stdout.contains(&0) {
        return Err(format!(
            "« {chemin} » est un fichier binaire : il ne se lit pas comme du texte."
        ));
    }
    let tout = String::from_utf8_lossy(&s.stdout);
    let n = tout.chars().count();
    let tronque = n > max || s.stdout.len() > max * 4;
    let contenu: String = tout.chars().take(max).collect();
    Ok(json!({
        "distro": distro,
        "path": chemin,
        "content": contenu,
        "truncated": tronque,
    }))
}

async fn write_file(args: &Value) -> Result<Value, String> {
    let chemin = requis(args, "path")?;
    let contenu = texte(args, "content").ok_or("paramètre « content » manquant")?;
    let ajout = args["append"] == json!(true);
    let distro = wsl::choisir(texte(args, "distro")).await?;
    let script = r#"mkdir -p -- "$(dirname -- "$1")" && if [ "$2" = a ]; then cat >> "$1"; else cat > "$1"; fi"#;
    let a = exec_args(
        &distro,
        None,
        false,
        &[
            "sh",
            "-c",
            script,
            "locaryn",
            chemin,
            if ajout { "a" } else { "w" },
        ],
    );
    let s = wsl::wsl(&a, Some(contenu.as_bytes()), Duration::from_secs(60)).await?;
    if s.code != Some(0) {
        return Err(format!(
            "Écriture impossible : {}",
            decode(&s.stderr).trim()
        ));
    }
    Ok(json!({
        "distro": distro,
        "path": chemin,
        "bytes": contenu.len(),
        "mode": if ajout { "append" } else { "write" },
    }))
}

async fn list_dir(args: &Value) -> Result<Value, String> {
    let chemin = texte(args, "path")
        .filter(|p| !p.trim().is_empty())
        .unwrap_or(".");
    let profondeur = args["depth"].as_u64().unwrap_or(1).clamp(1, 4).to_string();
    let distro = wsl::choisir(texte(args, "distro")).await?;
    let format = "%y\t%s\t%P\n";
    let a = exec_args(
        &distro,
        None,
        false,
        &[
            "find",
            chemin,
            "-mindepth",
            "1",
            "-maxdepth",
            &profondeur,
            "(",
            "-name",
            ".git",
            "-o",
            "-name",
            "node_modules",
            "-o",
            "-name",
            "target",
            "-o",
            "-name",
            "__pycache__",
            "-o",
            "-name",
            ".venv",
            "-o",
            "-name",
            "venv",
            "-o",
            "-name",
            "dist",
            "-o",
            "-name",
            ".cache",
            ")",
            "-printf",
            format,
            "-prune",
            "-o",
            "-printf",
            format,
        ],
    );
    let s = wsl::wsl(&a, None, Duration::from_secs(60)).await?;
    let sortie = decode(&s.stdout);
    if s.code != Some(0) && sortie.trim().is_empty() {
        return Err(format!(
            "Lecture du dossier impossible : {}",
            decode(&s.stderr).trim()
        ));
    }
    let mut entrees: Vec<Value> = sortie
        .lines()
        .filter_map(|l| {
            let mut c = l.splitn(3, '\t');
            let genre = c.next()?;
            let taille: u64 = c.next()?.parse().unwrap_or(0);
            let nom = c.next()?;
            Some(match genre {
                "d" => json!({ "type": "dir", "path": nom }),
                "l" => json!({ "type": "link", "path": nom }),
                _ => json!({ "type": "file", "path": nom, "size": taille }),
            })
        })
        .collect();
    entrees.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let total = entrees.len();
    entrees.truncate(ENTREES_MAX);
    Ok(json!({
        "distro": distro,
        "path": chemin,
        "entries": entrees,
        "truncated": total > ENTREES_MAX,
    }))
}

/// Un chemin Windows : `C:\…`, `C:/…` ou `\\serveur\…`.
pub fn est_windows(chemin: &str) -> bool {
    let b = chemin.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') || chemin.starts_with("\\\\")
}

async fn path(args: &Value) -> Result<Value, String> {
    let chemin = requis(args, "path")?;
    let distro = wsl::choisir(texte(args, "distro")).await?;
    let vers_linux = est_windows(chemin);
    let a = exec_args(
        &distro,
        None,
        false,
        &[
            "wslpath",
            "-a",
            if vers_linux { "-u" } else { "-w" },
            chemin,
        ],
    );
    let s = wsl::wsl(&a, None, Duration::from_secs(60)).await?;
    if s.code != Some(0) {
        return Err(format!(
            "Conversion impossible : {}",
            decode(&s.stderr).trim()
        ));
    }
    let converti = decode(&s.stdout).trim().to_string();
    Ok(if vers_linux {
        json!({ "distro": distro, "windows": chemin, "linux": converti })
    } else {
        json!({ "distro": distro, "linux": chemin, "windows": converti })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_commandes_destructrices_demandent_confirmation() {
        assert!(risque("rm -rf build").is_some());
        assert!(risque("cd x && sudo rm -r y").is_some());
        assert!(risque("git reset --hard HEAD~1").is_some());
        assert!(risque("docker system prune -a").is_some());
        assert!(risque("sudo apt-get remove nginx").is_some());
    }

    #[test]
    fn les_commandes_courantes_passent() {
        assert!(risque("cargo build --release").is_none());
        assert!(risque("npm run term").is_none());
        assert!(risque("ls -la ~/projets").is_none());
        assert!(risque("grep -rn 'firmware' .").is_none());
        assert!(risque("python3 -m pytest -q").is_none());
        assert!(risque("echo $TERM").is_none());
    }

    #[test]
    fn une_longue_sortie_garde_sa_fin() {
        let long = format!("{}FIN", "x".repeat(TETE + QUEUE + 100));
        let t = tronque(&long);
        assert!(t.ends_with("FIN"));
        assert!(t.contains("caractères omis"));
        assert_eq!(tronque("court\n"), "court");
    }

    #[test]
    fn les_avis_de_wsl_sont_mis_a_part() {
        let (reste, avis) = separer_avis_wsl(
            "wsl: Failed to start the systemd user session for 'a'.
erreur: x",
        );
        assert_eq!(reste, "erreur: x");
        assert_eq!(avis, ["Failed to start the systemd user session for 'a'."]);
    }

    #[test]
    fn reconnait_les_chemins_windows() {
        assert!(est_windows(r"C:\Users\a"));
        assert!(est_windows("d:/projets"));
        assert!(est_windows(r"\\wsl.localhost\Ubuntu\home"));
        assert!(!est_windows("/home/a"));
        assert!(!est_windows("~/projet"));
    }

    #[tokio::test]
    async fn as_root_demande_toujours_confirmation() {
        let r = call("wsl_run", json!({ "command": "id", "as_root": true }))
            .await
            .unwrap();
        assert_eq!(r["needs_confirmation"], true);
    }
}
