//! Les programmes interactifs : un script qui pose des questions, un
//! installateur qui attend « [Y/n] », un REPL.
//!
//! `wsl_run` attend la fin de la commande : un programme qui attend une
//! réponse le bloquait jusqu'au délai. Ici, `wsl_start` lance le programme et
//! rend la main dès qu'il se tait (il attend sans doute une entrée) ;
//! `wsl_send` lui écrit, `wsl_read` relit ce qu'il a écrit depuis, `wsl_stop`
//! l'arrête. Le programme tourne dans un pseudo-terminal (`script`), comme
//! dans une vraie console : les invites qui ne s'affichent qu'à un humain
//! (sudo, apt, read -p) apparaissent.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Sessions ouvertes au plus : chacune garde un processus WSL vivant.
const SESSIONS_MAX: usize = 8;
/// Durée de vie d'une session, sans exception.
const VIE_MAX_S: u64 = 3600;
/// Sortie gardée par session (la fin).
const TAMPON_MAX: usize = 400_000;
/// Silence après lequel on considère que le programme attend une entrée.
const SILENCE: Duration = Duration::from_millis(1500);

struct Session {
    _enfant: tokio::process::Child,
    entree: Option<tokio::process::ChildStdin>,
    tampon: Arc<Mutex<Vec<u8>>>,
    /// Octets déjà rendus au modèle.
    lu: usize,
    /// Code de sortie, une fois le programme terminé.
    fin: Arc<Mutex<Option<Option<i32>>>>,
    /// Dernière fois que le programme a écrit.
    vu: Arc<Mutex<Instant>>,
    distro: String,
    commande: String,
}

fn sessions() -> &'static tokio::sync::Mutex<HashMap<String, Session>> {
    static S: OnceLock<tokio::sync::Mutex<HashMap<String, Session>>> = OnceLock::new();
    S.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

fn texte<'a>(args: &'a Value, cle: &str) -> Option<&'a str> {
    args.get(cle).and_then(Value::as_str)
}

fn attente(args: &Value, defaut: u64) -> Duration {
    Duration::from_secs(args["wait_s"].as_u64().unwrap_or(defaut).clamp(1, 120))
}

/// Retirer les séquences d'un terminal (couleurs, curseur) et les retours
/// chariot : le modèle lit du texte, pas un écran.
pub fn nettoyer(brut: &str) -> String {
    let mut sortie = String::with_capacity(brut.len());
    let mut c = brut.chars().peekable();
    while let Some(x) = c.next() {
        match x {
            '\u{1b}' => match c.peek() {
                Some('[') => {
                    c.next();
                    for y in c.by_ref() {
                        if ('@'..='~').contains(&y) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    c.next();
                    for y in c.by_ref() {
                        if y == '\u{7}' {
                            break;
                        }
                    }
                }
                _ => {
                    c.next();
                }
            },
            '\r' => {
                if c.peek() != Some(&'\n') {
                    // Un retour chariot seul réécrit la ligne (barre de
                    // progression) : on passe à la ligne.
                    sortie.push('\n');
                }
            }
            _ => sortie.push(x),
        }
    }
    sortie
}

/// Ce qui met fin à l'attente sur une session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinAtteinte {
    /// Le programme s'est terminé (son code de sortie est rendu).
    Terminee,
    /// Il s'est tu sans finir : il attend sans doute une entrée.
    Invite,
    /// Le délai de l'appelant est écoulé (le programme peut encore tourner).
    Delai,
}

/// Attendre que le programme écrive puis se taise, se termine, ou que le
/// délai passe ; pendant ce temps, sa sortie part en direct à Locaryn.
async fn attendre(id: &str, delai: Duration) -> Result<Value, String> {
    attendre_avec(id, delai, None).await.map(|(v, _)| v)
}

/// Comme [`attendre`], mais rend aussi compte de la raison du réveil.
///
/// `invite`, fourni par `wsl_run`, reçoit la fin de la sortie (nettoyée) :
/// tant qu'il répond faux, un silence est un simple temps mort et on attend
/// la fin ou le délai — une compilation peut se taire un moment. S'il répond
/// vrai, la fin ressemble à une question (« Continue ? [Y/n] », mot de passe,
/// menu) : le modèle gagne à y répondre avec `wsl_send` plutôt qu'à attendre
/// que le délai tombe.
pub(crate) async fn attendre_avec(
    id: &str,
    delai: Duration,
    invite: Option<&dyn Fn(&str) -> bool>,
) -> Result<(Value, FinAtteinte), String> {
    let debut = Instant::now();
    let mut envoye = {
        let s = sessions().lock().await;
        s.get(id).map(|x| x.lu).ok_or_else(|| inconnue(id))?
    };
    let fin;
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (longueur, fini, vu) = {
            let s = sessions().lock().await;
            let x = s.get(id).ok_or_else(|| inconnue(id))?;
            let l = x.tampon.lock().map(|t| t.len()).unwrap_or(0);
            let fini = x.fin.lock().ok().and_then(|f| *f).is_some();
            let vu = x.vu.lock().map(|v| *v).unwrap_or_else(|_| Instant::now());
            (l, fini, vu)
        };
        if longueur > envoye {
            let nouveau = {
                let s = sessions().lock().await;
                let x = s.get(id).ok_or_else(|| inconnue(id))?;
                let t = x.tampon.lock().unwrap();
                String::from_utf8_lossy(&t[envoye.min(t.len())..]).into_owned()
            };
            crate::mcp::progres(&crate::tools::sans_avis_wsl(&nettoyer(&nouveau)));
            envoye = longueur;
        }
        if fini {
            fin = FinAtteinte::Terminee;
            break;
        }
        if debut.elapsed() >= delai {
            fin = FinAtteinte::Delai;
            break;
        }
        let a_ecrit = longueur > 0;
        if a_ecrit && vu.elapsed() >= SILENCE && debut.elapsed() >= SILENCE {
            let prend = match invite {
                None => true,
                Some(f) => {
                    // La fin de la sortie, telle que le modèle la lira.
                    let queue = {
                        let s = sessions().lock().await;
                        let x = s.get(id).ok_or_else(|| inconnue(id))?;
                        let t = x.tampon.lock().unwrap();
                        let debut_queue = t.len().saturating_sub(2_000);
                        String::from_utf8_lossy(&t[debut_queue..]).into_owned()
                    };
                    f(&nettoyer(&queue))
                }
            };
            if prend {
                fin = FinAtteinte::Invite;
                break;
            }
        }
    }
    Ok((rendre(id).await?, fin))
}

fn inconnue(id: &str) -> String {
    format!("session « {id} » inconnue ou terminée : relancez le programme avec wsl_start")
}

/// Ce qui est nouveau depuis la dernière lecture, et l'état du programme.
async fn rendre(id: &str) -> Result<Value, String> {
    let mut s = sessions().lock().await;
    let x = s.get_mut(id).ok_or_else(|| inconnue(id))?;
    let (nouveau, total) = {
        let t = x.tampon.lock().unwrap();
        (
            String::from_utf8_lossy(&t[x.lu.min(t.len())..]).into_owned(),
            t.len(),
        )
    };
    x.lu = total;
    // Les flux sont fermés mais le code n'est pas encore lu : le processus
    // finit de sortir, on l'attend un instant plutôt que de rendre « null ».
    if x.fin.lock().ok().and_then(|f| *f) == Some(None) {
        if let Ok(Ok(statut)) =
            tokio::time::timeout(Duration::from_millis(1500), x._enfant.wait()).await
        {
            if let Ok(mut f) = x.fin.lock() {
                *f = Some(statut.code());
            }
        }
    }
    let fin = x.fin.lock().ok().and_then(|f| *f);
    let silencieux = x.vu.lock().unwrap().elapsed() >= SILENCE;
    // Les avis de WSL (« wsl: Failed to start the systemd user session… »)
    // ne viennent pas de la commande : à part, comme dans wsl_run.
    let propre = nettoyer(&nouveau);
    let (reste, avis) = crate::tools::separer_avis_wsl(&propre);
    let mut r = json!({
        "session": id,
        "distro": x.distro,
        "command": x.commande,
        "output": crate::tools::tronque(&reste),
        "running": fin.is_none(),
    });
    if !avis.is_empty() {
        r["wsl_notice"] = json!(avis.join("\n"));
    }
    match fin {
        Some(code) => {
            r["exit_code"] = json!(code);
            s.remove(id);
        }
        None if silencieux => {
            r["note"] = json!("Le programme attend sans doute une entrée : répondez avec wsl_send (input), relisez avec wsl_read, ou arrêtez-le avec wsl_stop.");
        }
        None => {
            r["note"] = json!("Le programme travaille encore : relisez la suite avec wsl_read.");
        }
    }
    Ok(r)
}

pub async fn start(args: &Value) -> Result<Value, String> {
    let commande = texte(args, "command")
        .filter(|c| !c.trim().is_empty())
        .ok_or("command est obligatoire")?;
    if args["confirmed"] != json!(true) {
        if let Some(motif) = crate::tools::catastrophique(commande) {
            return Ok(json!({
                "refused": true,
                "reason": format!("commande qui détruirait le système ou un disque (« {} »)", motif.trim()),
                "next": "Non lancée. Si l'utilisateur la veut vraiment, rappelez avec confirmed=true."
            }));
        }
    }
    let root = args["as_root"] == json!(true);
    let distro = crate::wsl::choisir(texte(args, "distro")).await?;
    let dossier = texte(args, "cwd").filter(|c| !c.trim().is_empty());
    let id = lancer(
        commande,
        distro,
        dossier,
        root,
        texte(args, "sudo_password")
            .filter(|p| !p.is_empty())
            .map(str::to_string),
        VIE_MAX_S,
    )
    .await?;
    attendre(&id, attente(args, 5)).await
}

/// Lancer une commande dans un pseudo-terminal et rendre l'identifiant de sa
/// session : le corps commun de `wsl_start` et de `wsl_run`. `delai_s` borne
/// la commande elle-même (par `timeout -k 5`), pas l'attente de l'appelant :
/// c'est ce qui laisse une session vivante après un retour de main.
pub(crate) async fn lancer(
    commande: &str,
    distro: String,
    dossier: Option<&str>,
    root: bool,
    mot_de_passe: Option<String>,
    delai_s: u64,
) -> Result<String, String> {
    {
        let mut s = sessions().lock().await;
        // Les sessions terminées et déjà relues n'occupent plus de place.
        s.retain(|_, x| x.fin.lock().ok().and_then(|f| *f).is_none());
        if s.len() >= SESSIONS_MAX {
            return Err(format!(
                "{SESSIONS_MAX} programmes interactifs tournent déjà : arrêtez-en un avec wsl_stop"
            ));
        }
    }
    let vie = delai_s.to_string();
    // Un pseudo-terminal quand `script` est là (util-linux, présent sur
    // Ubuntu/Debian/Kali…), sinon un shell simple.
    let lanceur = r#"if command -v script >/dev/null 2>&1; then exec script -qfec "$1" /dev/null; elif command -v bash >/dev/null 2>&1; then exec bash -lc "$1"; else exec sh -lc "$1"; fi"#;
    if mot_de_passe
        .as_deref()
        .is_some_and(|p| p.contains("{{secret:"))
    {
        return Err(
            "sudo_password n'a pas été rempli par le coffre : mettez Locaryn à jour.".into(),
        );
    }
    let avec_sudo = mot_de_passe.is_some() && crate::tools::utilise_sudo(commande);
    // `stty -echo` d'abord : le pseudo-terminal réaffiche tout ce qu'on lui
    // écrit, et un mot de passe sudo ne doit jamais revenir dans la sortie.
    let lancee = if avec_sudo {
        format!(
            "stty -echo 2>/dev/null; {}",
            crate::tools::prefixe_sudo(commande)
        )
    } else {
        commande.to_string()
    };
    let a = crate::wsl::exec_args(
        &distro,
        dossier,
        root,
        &[
            "timeout", "-k", "5", &vie, "sh", "-c", lanceur, "locaryn", &lancee,
        ],
    );
    let mut cmd = tokio::process::Command::new("wsl.exe");
    cmd.args(&a)
        .env("WSL_UTF8", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let mut enfant = cmd
        .spawn()
        .map_err(|e| format!("wsl.exe introuvable ({e})"))?;
    let entree = enfant.stdin.take();
    let tampon = Arc::new(Mutex::new(Vec::new()));
    let vu = Arc::new(Mutex::new(Instant::now()));
    let fin = Arc::new(Mutex::new(None));
    let flux: Vec<Box<dyn tokio::io::AsyncRead + Unpin + Send>> = vec![
        Box::new(enfant.stdout.take().ok_or("stdout indisponible")?),
        Box::new(enfant.stderr.take().ok_or("stderr indisponible")?),
    ];
    let lecteurs = Arc::new(std::sync::atomic::AtomicUsize::new(flux.len()));
    for mut f in flux {
        let (tampon, vu, fin, lecteurs) =
            (tampon.clone(), vu.clone(), fin.clone(), lecteurs.clone());
        tokio::spawn(async move {
            let mut buf = [0u8; 8192];
            while let Ok(n) = f.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                if let Ok(mut t) = tampon.lock() {
                    t.extend_from_slice(&buf[..n]);
                    if t.len() > TAMPON_MAX {
                        let trop = t.len() - TAMPON_MAX;
                        t.drain(..trop);
                    }
                }
                if let Ok(mut v) = vu.lock() {
                    *v = Instant::now();
                }
            }
            // Le dernier flux fermé : le programme est terminé.
            if lecteurs.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                if let Ok(mut f) = fin.lock() {
                    f.get_or_insert(None);
                }
            }
        });
    }
    let id = format!("s{}", rand_id());
    sessions().lock().await.insert(
        id.clone(),
        Session {
            _enfant: enfant,
            entree,
            tampon,
            lu: 0,
            fin,
            vu,
            distro,
            commande: commande.to_string(),
        },
    );
    // Le code de sortie, une fois les flux fermés.
    {
        let id = id.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let mut s = sessions().lock().await;
                let Some(x) = s.get_mut(&id) else { return };
                if let Ok(Some(statut)) = x._enfant.try_wait() {
                    if let Ok(mut f) = x.fin.lock() {
                        *f = Some(statut.code());
                    }
                    return;
                }
            }
        });
    }
    // Le mot de passe sudo part une fois le terminal configuré
    // (`stty -echo` ci-dessus) : écrit plus tôt, le pseudo-terminal le
    // réafficherait dans la sortie de la commande.
    if avec_sudo {
        let p = mot_de_passe.unwrap_or_default();
        tokio::time::sleep(Duration::from_millis(400)).await;
        let mut s = sessions().lock().await;
        if let Some(x) = s.get_mut(&id) {
            if let Some(e) = x.entree.as_mut() {
                let _ = e.write_all(format!("{p}\n").as_bytes()).await;
                let _ = e.flush().await;
            }
        }
    }
    Ok(id)
}

fn rand_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{:x}{}", t & 0xffff, N.fetch_add(1, Ordering::Relaxed) + 1)
}

/// Une touche nommée en sa séquence pour le terminal : flèches, espace,
/// tabulation… — de quoi cocher une case dans un menu `whiptail`/`dialog`
/// (flèches pour se déplacer, espace pour (dé)cocher, tab vers OK, entrée).
fn touche_vers_octets(nom: &str) -> Option<&'static [u8]> {
    Some(match nom.trim().to_ascii_lowercase().as_str() {
        "up" | "haut" => b"\x1b[A",
        "down" | "bas" => b"\x1b[B",
        "right" | "droite" => b"\x1b[C",
        "left" | "gauche" => b"\x1b[D",
        "space" | "espace" => b" ",
        "tab" => b"\t",
        // La touche Entrée, c'est le retour chariot : un menu whiptail/dialog
        // en mode brut n'attend que lui. En mode ligne (read -p), le terminal
        // le convertit en saut de ligne — il marche donc dans les deux cas.
        "enter" | "return" | "entree" | "entr\u{e9}e" => b"\r",
        "esc" | "escape" | "echap" => b"\x1b",
        "backspace" | "retour" => b"\x7f",
        "home" => b"\x1b[H",
        "end" | "fin" => b"\x1b[F",
        "pageup" => b"\x1b[5~",
        "pagedown" => b"\x1b[6~",
        "ctrl-c" | "ctrl_c" => b"\x03",
        _ => return None,
    })
}

pub async fn send(args: &Value) -> Result<Value, String> {
    let id = texte(args, "session").ok_or("session est obligatoire (rendue par wsl_start)")?;
    let mut octets: Vec<u8> = Vec::new();
    if args["ctrl_c"] == json!(true) {
        octets.push(0x03);
    } else if let Some(touches) = args.get("keys").and_then(Value::as_array) {
        // Une suite de touches : naviguer dans un menu (flèches, espace, tab…).
        for t in touches {
            let Some(nom) = t.as_str() else { continue };
            match touche_vers_octets(nom) {
                Some(seq) => octets.extend_from_slice(seq),
                None => return Err(format!(
                    "touche inconnue « {nom} » : up, down, left, right, space, tab, enter, esc, backspace, home, end, pageup, pagedown"
                )),
            }
        }
    } else {
        let saisie = texte(args, "input").unwrap_or("");
        if saisie.contains("{{secret:") {
            return Err("input n'a pas été rempli par le coffre : mettez Locaryn à jour.".into());
        }
        octets.extend_from_slice(saisie.as_bytes());
        if args["enter"] != json!(false) {
            // Retour chariot : Entrée pour un programme en mode brut (TUI) comme
            // en mode ligne (read -p, que le terminal convertit en saut de ligne).
            octets.push(b'\r');
        }
    }
    {
        let mut s = sessions().lock().await;
        let x = s.get_mut(id).ok_or_else(|| inconnue(id))?;
        let e = x
            .entree
            .as_mut()
            .ok_or("l'entrée du programme est fermée")?;
        e.write_all(&octets)
            .await
            .map_err(|e| format!("écriture refusée : {e} (le programme s'est peut-être arrêté)"))?;
        let _ = e.flush().await;
        if let Ok(mut v) = x.vu.lock() {
            *v = Instant::now();
        };
    }
    attendre(id, attente(args, 5)).await
}

pub async fn read(args: &Value) -> Result<Value, String> {
    let id = texte(args, "session").ok_or("session est obligatoire (rendue par wsl_start)")?;
    attendre(id, attente(args, 5)).await
}

pub async fn stop(args: &Value) -> Result<Value, String> {
    let id = texte(args, "session").ok_or("session est obligatoire")?;
    let derniere = rendre(id).await.ok();
    sessions().lock().await.remove(id);
    Ok(
        json!({ "session": id, "stopped": true, "last_output": derniere.map(|d| d["output"].clone()) }),
    )
}

#[cfg(test)]
mod tests {
    use super::nettoyer;

    #[test]
    fn les_touches_de_navigation_deviennent_des_sequences() {
        assert_eq!(super::touche_vers_octets("down"), Some(&b"\x1b[B"[..]));
        assert_eq!(super::touche_vers_octets("space"), Some(&b" "[..]));
        assert_eq!(super::touche_vers_octets("Tab"), Some(&b"\t"[..]));
        assert!(super::touche_vers_octets("f13").is_none());
    }

    #[test]
    fn la_sortie_d_un_terminal_devient_du_texte() {
        assert_eq!(
            nettoyer("\u{1b}[1;32mOK\u{1b}[0m\r\nDo you want to continue? [Y/n] "),
            "OK\nDo you want to continue? [Y/n] "
        );
        // Une barre de progression qui se réécrit passe à la ligne.
        assert_eq!(nettoyer("10%\r50%\r100%\r\n"), "10%\n50%\n100%\n");
        assert_eq!(nettoyer("\u{1b}]0;titre\u{7}prompt$ "), "prompt$ ");
    }
}
