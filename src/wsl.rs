//! Appels à `wsl.exe` : lister les distributions, exécuter dans l'une d'elles.
//!
//! Tout passe par `wsl.exe --exec`, qui lance le programme Linux sans passer
//! par un shell : la commande de l'utilisateur et les chemins voyagent comme
//! arguments, jamais recollés dans une ligne à interpréter. Seul `wsl_run`
//! confie volontairement sa commande à `bash -lc`.

use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Une distribution installée, telle que `wsl -l -v` la décrit.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Distro {
    pub name: String,
    pub state: String,
    pub version: u8,
    pub default: bool,
}

/// Ce qu'un programme a rendu.
pub struct Sortie {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// `wsl.exe` écrit ses propres messages en UTF-16 (sauf avec `WSL_UTF8=1`,
/// que les versions anciennes ignorent), les programmes Linux en UTF-8.
pub fn decode(octets: &[u8]) -> String {
    let octets = octets.strip_prefix(&[0xFF, 0xFE]).unwrap_or(octets);
    let impairs_nuls = octets
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|&&b| b == 0)
        .count();
    if octets.len() >= 2 && impairs_nuls * 2 >= octets.len() / 2 {
        let unites: Vec<u16> = octets
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&unites).replace('\0', "");
    }
    String::from_utf8_lossy(octets).into_owned()
}

/// Lance `wsl.exe` avec ces arguments, sans fenêtre de console.
pub async fn wsl(
    args: &[String],
    entree: Option<&[u8]>,
    delai: Duration,
) -> Result<Sortie, String> {
    wsl_suivi(args, entree, delai, None).await
}

/// Ce qui reçoit la sortie au fil de l'eau.
pub type Suivi<'a> = Option<&'a (dyn Fn(&str) + Sync)>;

/// Prendre dans `attente` ce qui forme de l'UTF-8 complet, et garder la fin
/// d'un caractère coupé entre deux lectures pour la suivante.
pub fn prendre_utf8(attente: &mut Vec<u8>) -> String {
    let complet = match std::str::from_utf8(attente) {
        Ok(_) => attente.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => attente.len(),
    };
    let reste = attente.split_off(complet);
    let texte = String::from_utf8_lossy(attente).into_owned();
    *attente = reste;
    texte
}

/// Comme [`wsl`], en passant la sortie à `suivi` pendant que la commande
/// tourne (quelques fois par seconde) : une mise à jour se suit en direct.
pub async fn wsl_suivi(
    args: &[String],
    entree: Option<&[u8]>,
    delai: Duration,
    suivi: Suivi<'_>,
) -> Result<Sortie, String> {
    let mut cmd = tokio::process::Command::new("wsl.exe");
    cmd.args(args)
        .env("WSL_UTF8", "1")
        .stdin(if entree.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut enfant = cmd.spawn().map_err(|e| {
        format!("wsl.exe introuvable ({e}). WSL s'installe avec « wsl --install » dans un terminal administrateur.")
    })?;
    if let (Some(donnees), Some(mut stdin)) = (entree, enfant.stdin.take()) {
        let donnees = donnees.to_vec();
        tokio::spawn(async move {
            if let Err(e) = stdin.write_all(&donnees).await {
                eprintln!("wsl : écriture sur l'entrée : {e}");
            }
            drop(stdin);
        });
    }
    let out = enfant.stdout.take().ok_or("stdout indisponible")?;
    let err = enfant.stderr.take().ok_or("stderr indisponible")?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(bool, Vec<u8>)>();
    for (est_err, mut flux) in [
        (
            false,
            Box::new(out) as Box<dyn tokio::io::AsyncRead + Unpin + Send>,
        ),
        (true, Box::new(err)),
    ] {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut tampon = [0u8; 8192];
            while let Ok(n) = flux.read(&mut tampon).await {
                if n == 0 || tx.send((est_err, tampon[..n].to_vec())).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);
    let lecture = async {
        let (mut o, mut e) = (Vec::new(), Vec::new());
        let mut attente: Vec<u8> = Vec::new();
        let mut cadence = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                morceau = rx.recv() => match morceau {
                    Some((est_err, octets)) => {
                        if suivi.is_some() {
                            attente.extend_from_slice(&octets);
                        }
                        if est_err { e.extend(octets) } else { o.extend(octets) }
                    }
                    None => break,
                },
                _ = cadence.tick() => {
                    if let Some(f) = suivi {
                        let texte = prendre_utf8(&mut attente);
                        if !texte.is_empty() {
                            f(&texte);
                        }
                    }
                }
            }
        }
        if let Some(f) = suivi {
            let texte = String::from_utf8_lossy(&attente).into_owned();
            if !texte.is_empty() {
                f(&texte);
            }
        }
        let statut = enfant.wait().await.map_err(|x| format!("attente : {x}"))?;
        Ok::<_, String>(Sortie {
            code: statut.code(),
            stdout: o,
            stderr: e,
        })
    };
    tokio::time::timeout(delai, lecture)
        .await
        .map_err(|_| format!("WSL n'a pas répondu en {} s", delai.as_secs()))?
}

/// Les distributions installées.
pub async fn distributions() -> Result<Vec<Distro>, String> {
    let s = wsl(&["-l".into(), "-v".into()], None, Duration::from_secs(60)).await?;
    let texte = decode(&s.stdout);
    if s.code != Some(0) {
        let detail = format!("{} {}", texte.trim(), decode(&s.stderr).trim());
        return Err(format!(
            "Aucune distribution WSL utilisable : {}. Installez-en une avec « wsl --install -d Ubuntu ».",
            detail.trim()
        ));
    }
    Ok(analyser_liste(&texte))
}

/// Lit le tableau de `wsl -l -v` (« * Ubuntu  Running  2 »).
pub fn analyser_liste(texte: &str) -> Vec<Distro> {
    texte
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("NAME") && !l.contains("NOM"))
        .skip(1)
        .filter_map(|ligne| {
            let defaut = ligne.trim_start().starts_with('*');
            let mut champs = ligne
                .trim_start()
                .trim_start_matches('*')
                .split_whitespace();
            let name = champs.next()?.to_string();
            let state = champs.next().unwrap_or("").to_string();
            let version = champs.next().and_then(|v| v.parse().ok()).unwrap_or(2);
            Some(Distro {
                name,
                state,
                version,
                default: defaut,
            })
        })
        .collect()
}

/// La distribution demandée, ou celle par défaut.
pub async fn choisir(demandee: Option<&str>) -> Result<String, String> {
    let liste = distributions().await?;
    if let Some(nom) = demandee.filter(|n| !n.trim().is_empty()) {
        return liste
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(nom.trim()))
            .map(|d| d.name.clone())
            .ok_or_else(|| {
                let noms: Vec<&str> = liste.iter().map(|d| d.name.as_str()).collect();
                format!(
                    "Distribution « {nom} » introuvable. Installées : {}",
                    noms.join(", ")
                )
            });
    }
    liste
        .iter()
        .find(|d| d.default)
        .or_else(|| liste.first())
        .map(|d| d.name.clone())
        .ok_or_else(|| {
            "Aucune distribution WSL installée. « wsl --install -d Ubuntu » en installe une.".into()
        })
}

/// Les arguments qui exécutent `programme` dans `distro`, dans `dossier`.
pub fn exec_args(
    distro: &str,
    dossier: Option<&str>,
    root: bool,
    programme: &[&str],
) -> Vec<String> {
    let mut a: Vec<String> = vec!["-d".into(), distro.into()];
    if root {
        a.extend(["-u".into(), "root".into()]);
    }
    a.extend([
        "--cd".into(),
        dossier
            .filter(|d| !d.trim().is_empty())
            .unwrap_or("~")
            .into(),
    ]);
    a.push("--exec".into());
    a.extend(programme.iter().map(|s| s.to_string()));
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lit_la_liste_en_utf16() {
        let texte = "  NAME            STATE           VERSION\r\n* Ubuntu          Stopped         2\r\n  Ubuntu-22.04    Running         2\r\n  docker-desktop  Stopped         1\r\n";
        let mut octets = vec![0xFF, 0xFE];
        for u in texte.encode_utf16() {
            octets.extend(u.to_le_bytes());
        }
        let liste = analyser_liste(&decode(&octets));
        assert_eq!(liste.len(), 3);
        assert_eq!(liste[0].name, "Ubuntu");
        assert!(liste[0].default);
        assert_eq!(liste[1].state, "Running");
        assert!(!liste[1].default);
        assert_eq!(liste[2].version, 1);
    }

    #[test]
    fn un_caractere_coupe_attend_la_lecture_suivante() {
        let mut a = "é".as_bytes()[..1].to_vec();
        assert_eq!(prendre_utf8(&mut a), "");
        a.extend_from_slice(&"é".as_bytes()[1..]);
        a.extend_from_slice(b" ok");
        assert_eq!(prendre_utf8(&mut a), "é ok");
        assert!(a.is_empty());
    }

    #[test]
    fn l_utf8_reste_de_l_utf8() {
        assert_eq!(decode("bonjour é".as_bytes()), "bonjour é");
    }

    #[test]
    fn les_arguments_ne_passent_pas_par_un_shell() {
        let a = exec_args("Ubuntu", None, false, &["cat", "--", "/tmp/a b"]);
        assert_eq!(
            a,
            ["-d", "Ubuntu", "--cd", "~", "--exec", "cat", "--", "/tmp/a b"]
        );
        let r = exec_args("Ubuntu", Some("/srv"), true, &["id"]);
        assert_eq!(
            r,
            ["-d", "Ubuntu", "-u", "root", "--cd", "/srv", "--exec", "id"]
        );
    }
}
