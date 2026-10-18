//! Les outils MCP du morph : distributions, commandes, fichiers, chemins.
//!
//! Le modèle travaille dans WSL comme dans un terminal : commandes, lecture,
//! écriture, modification, déplacement et suppression de fichiers, en root
//! quand il le faut. L'accord de l'utilisateur passe par Locaryn (fenêtre
//! d'approbation des outils, selon le niveau de permission choisi) ; le morph
//! ne bloque lui-même que les commandes qui détruiraient le système.

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

/// Ce qui détruirait la distribution ou un disque : refusé sans
/// `confirmed=true`. Tout le reste (rm d'un fichier, apt remove, git reset…)
/// passe par l'approbation de Locaryn, sans second aller-retour : les petits
/// modèles s'y perdaient.
const COMMANDES_CATASTROPHIQUES: &[&str] = &[
    "rm -rf / ",
    "rm -rf /*",
    "rm -fr / ",
    "rm -rf ~ ",
    "rm -rf --no-preserve-root",
    "mkfs",
    "wipefs",
    "of=/dev/sd",
    "of=/dev/nvme",
    "> /dev/sd",
    ":(){",
    "chmod -r 777 / ",
    "chown -r ",
    "wsl.exe --unregister",
    "wsl --unregister",
];

/// Dossiers qu'aucune suppression ne vise d'un coup.
const RACINES_PROTEGEES: &[&str] = &[
    "/", "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64", "/mnt", "/opt", "/proc",
    "/root", "/sbin", "/sys", "/usr", "/var", "~", "/mnt/c", "/mnt/d",
];

fn annote(mut t: Value, annotations: Value) -> Value {
    t["annotations"] = annotations;
    t
}

pub fn tools_list() -> Value {
    let distro = || str_prop("Distribution (wsl_distros). Omise : celle par défaut.");
    let root = || json!({ "type": "boolean", "description": "Exécuter en root (fichiers système, /etc…), dans le dossier de l'utilisateur par défaut." });
    let lecture = json!({ "readOnlyHint": true, "openWorldHint": false });
    json!({ "tools": [
        annote(tool(
            "wsl_distros",
            "Liste les distributions WSL installées (nom, état, version, celle par défaut).",
            json!({}),
            &[],
        ), lecture.clone()),
        tool(
            "wsl_run",
            "Exécute une commande bash dans une distribution WSL, en tant que l'utilisateur, et rend sa sortie (code, stdout, stderr). Si la commande se met à attendre une réponse (question [Y/n], menu, mot de passe), la main vous est rendue avec un identifiant de session : répondez par wsl_send, relisez par wsl_read. Préférez malgré tout -y, --no-pager, DEBIAN_FRONTEND=noninteractive. Si sudo demande un mot de passe : sudo_password = {{secret:wsl-sudo-<distro>}} (le mot de passe vient du coffre de Locaryn, vous ne le voyez jamais), ou wsl_allow_sudo.",
            json!({
                "command": str_prop("Commande bash"),
                "distro": distro(),
                "cwd": str_prop("Dossier de travail : chemin Linux (/home/…, ~/projet) ou Windows (C:\\…). Défaut : ~"),
                "timeout_s": { "type": "integer", "description": "Délai en secondes (défaut 120, max 1800)" },
                "as_root": root(),
                "sudo_password": str_prop("Mot de passe sudo : écrivez exactement {{secret:wsl-sudo-<distro>}}, jamais le mot de passe lui-même — Locaryn y met la valeur du coffre. Pas encore dans le coffre : demandez-le avec ask_secret (name=\"wsl-sudo-<distro>\")."),
            }),
            &["command"],
        ),
        tool(
            "wsl_start",
            "Lance un programme interactif (script qui pose des questions, installateur « [Y/n] », REPL, assistant de configuration) dans un pseudo-terminal WSL, et rend la main dès qu'il attend une entrée — sans bloquer jusqu'à la fin. Rend un identifiant de session : répondez avec wsl_send, relisez avec wsl_read, arrêtez avec wsl_stop. Pour une commande qui ne demande rien, wsl_run suffit.",
            json!({
                "command": str_prop("Commande à lancer"),
                "distro": distro(),
                "cwd": str_prop("Dossier de travail (défaut : ~)"),
                "as_root": root(),
                "sudo_password": str_prop("Si la commande utilise sudo : {{secret:wsl-sudo-<distro>}}, jamais le mot de passe lui-même."),
                "wait_s": { "type": "integer", "description": "Attente maximale avant de rendre la main (défaut 5 s, max 120). Rend plus tôt si le programme se tait." },
            }),
            &["command"],
        ),
        tool(
            "wsl_send",
            "Répond au programme d'une session (wsl_start), puis rend ce qu'il affiche ensuite. Soit du texte (input, Entrée ajoutée par défaut), soit une suite de touches (keys) pour un menu en plein écran whiptail/dialog : flèches pour se déplacer, « space » pour cocher/décocher une case, « tab » pour aller sur OK, « enter » pour valider. Un mot de passe demandé par le programme : input = {{secret:nom}} (voir ask_secret), jamais en clair.",
            json!({
                "session": str_prop("Identifiant rendu par wsl_start"),
                "input": str_prop("Texte à taper (ex. « y », « 2 », un nom). Ignoré si keys est fourni."),
                "keys": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Suite de touches pour un menu : up, down, left, right, space, tab, enter, esc, backspace, home, end, pageup, pagedown. Ex. cocher la 2e case puis valider : [\"down\",\"space\",\"tab\",\"enter\"]."
                },
                "enter": { "type": "boolean", "description": "Appuyer sur Entrée après le texte (défaut true). Sans effet avec keys." },
                "ctrl_c": { "type": "boolean", "description": "Envoyer Ctrl+C au lieu d'un texte" },
                "wait_s": { "type": "integer", "description": "Attente maximale (défaut 5 s)" },
            }),
            &["session"],
        ),
        annote(tool(
            "wsl_read",
            "Relit ce que le programme d'une session a affiché depuis la dernière lecture (il travaille encore, ou il a fini).",
            json!({
                "session": str_prop("Identifiant rendu par wsl_start"),
                "wait_s": { "type": "integer", "description": "Attente maximale de nouvelle sortie (défaut 5 s)" },
            }),
            &["session"],
        ), lecture.clone()),
        tool(
            "wsl_stop",
            "Arrête le programme d'une session et rend sa dernière sortie.",
            json!({ "session": str_prop("Identifiant rendu par wsl_start") }),
            &["session"],
        ),
        annote(tool(
            "wsl_read_file",
            "Lit un fichier texte d'une distribution WSL (chemin Linux, relatif à ~ s'il ne commence pas par /).",
            json!({
                "path": str_prop("Chemin Linux du fichier"),
                "distro": distro(),
                "max_chars": { "type": "integer", "description": "Taille lue au plus (défaut 100000)" },
                "as_root": root(),
            }),
            &["path"],
        ), lecture.clone()),
        tool(
            "wsl_write_file",
            "Crée ou remplace un fichier texte dans une distribution WSL (dossiers créés au besoin ; append pour ajouter à la fin).",
            json!({
                "path": str_prop("Chemin Linux du fichier"),
                "content": str_prop("Contenu complet du fichier"),
                "distro": distro(),
                "append": { "type": "boolean", "description": "Ajouter à la fin au lieu de remplacer" },
                "as_root": root(),
            }),
            &["path", "content"],
        ),
        tool(
            "wsl_edit_file",
            "Modifie un fichier d'une distribution WSL en remplaçant un passage exact (old) par un autre (new), sans réécrire tout le fichier.",
            json!({
                "path": str_prop("Chemin Linux du fichier"),
                "old": str_prop("Le passage actuel, tel quel (il doit être unique, sauf avec replace_all)"),
                "new": str_prop("Le nouveau passage"),
                "replace_all": { "type": "boolean", "description": "Remplacer toutes les occurrences" },
                "distro": distro(),
                "as_root": root(),
            }),
            &["path", "old", "new"],
        ),
        annote(tool(
            "wsl_delete",
            "Supprime un fichier, ou un dossier avec recursive=true, dans une distribution WSL. Les dossiers système et personnels entiers sont refusés.",
            json!({
                "path": str_prop("Chemin Linux à supprimer"),
                "recursive": { "type": "boolean", "description": "Supprimer un dossier et son contenu" },
                "distro": distro(),
                "as_root": root(),
            }),
            &["path"],
        ), json!({ "destructiveHint": true })),
        tool(
            "wsl_move",
            "Déplace ou renomme un fichier ou un dossier dans une distribution WSL (dossiers de destination créés au besoin).",
            json!({
                "from": str_prop("Chemin Linux actuel"),
                "to": str_prop("Nouveau chemin Linux"),
                "distro": distro(),
                "as_root": root(),
            }),
            &["from", "to"],
        ),
        tool(
            "wsl_allow_sudo",
            "Autorise sudo sans mot de passe pour l'utilisateur d'une distribution WSL (fichier /etc/sudoers.d/90-locaryn). Une fois, avec l'accord explicite de l'utilisateur : demandez-le, puis appelez avec confirmed=true. Ensuite, sudo fonctionne dans wsl_run.",
            json!({
                "distro": distro(),
                "confirmed": { "type": "boolean", "description": "true seulement après accord explicite de l'utilisateur" },
            }),
            &[],
        ),
        annote(tool(
            "wsl_list_dir",
            "Liste un dossier d'une distribution WSL (type, taille, chemin). .git, node_modules, target… sont cités sans être parcourus.",
            json!({
                "path": str_prop("Chemin Linux du dossier. Défaut : ~"),
                "distro": distro(),
                "depth": { "type": "integer", "description": "Profondeur (1 à 4, défaut 1)" },
            }),
            &[],
        ), lecture.clone()),
        annote(tool(
            "wsl_path",
            "Convertit un chemin entre Windows et WSL : C:\\Users\\… → /mnt/c/Users/…, et /home/… → \\\\wsl.localhost\\Distro\\home\\… (ouvrable depuis Windows).",
            json!({ "path": str_prop("Chemin Windows ou Linux"), "distro": distro() }),
            &["path"],
        ), lecture),
    ]})
}

pub async fn call(name: &str, args: Value) -> Result<Value, String> {
    match name {
        "wsl_distros" => distros().await,
        "wsl_run" => run(&args).await,
        "wsl_start" => crate::sessions::start(&args).await,
        "wsl_send" => crate::sessions::send(&args).await,
        "wsl_read" => crate::sessions::read(&args).await,
        "wsl_stop" => crate::sessions::stop(&args).await,
        "wsl_read_file" => read_file(&args).await,
        "wsl_write_file" => write_file(&args).await,
        "wsl_edit_file" => edit_file(&args).await,
        "wsl_delete" => delete(&args).await,
        "wsl_move" => move_path(&args).await,
        "wsl_allow_sudo" => allow_sudo(&args).await,
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

fn en_root(args: &Value) -> bool {
    args["as_root"] == json!(true)
}

async fn distros() -> Result<Value, String> {
    let liste = wsl::distributions().await?;
    Ok(json!({ "distros": liste }))
}

/// La commande détruirait-elle le système ou un disque ?
pub fn catastrophique(commande: &str) -> Option<&'static str> {
    let bas = format!(" {} ", commande.to_lowercase().replace('\t', " "));
    let bas = bas.replace(['\n', ';', '&', '|', '(', ')', '`'], " ");
    COMMANDES_CATASTROPHIQUES
        .iter()
        .copied()
        .find(|motif| bas.contains(motif))
}

/// La sortie en direct sans les avis de WSL lui-même (« wsl: Failed to start
/// the systemd user session… ») : ils ne viennent pas de la commande.
pub fn sans_avis_wsl(t: &str) -> String {
    t.split_inclusive('\n')
        .filter(|l| !l.starts_with("wsl: "))
        .collect()
}

/// Valider sudo une fois avec le mot de passe lu sur l'entrée standard, puis
/// lancer la commande : ses `sudo` profitent de l'autorisation fraîche, sans
/// rien lire de plus.
pub fn prefixe_sudo(commande: &str) -> String {
    format!("sudo -S -p '' -v >/dev/null 2>&1; {commande}")
}

/// La commande appelle-t-elle sudo (en début de mot) ?
pub fn utilise_sudo(commande: &str) -> bool {
    commande
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | '`'))
        .any(|mot| mot == "sudo")
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
    if args["confirmed"] != json!(true) {
        if let Some(motif) = catastrophique(commande) {
            return Ok(json!({
                "refused": true,
                "command": commande,
                "reason": format!("commande qui détruirait le système ou un disque (« {} »)", motif.trim()),
                "next": "Non exécutée. Si l'utilisateur la veut vraiment, il doit le confirmer explicitement : rappelez alors l'outil avec confirmed=true."
            }));
        }
    }
    let root = en_root(args);
    let distro = wsl::choisir(texte(args, "distro")).await?;
    // En root, `~` serait /root : on part du dossier de l'utilisateur, comme
    // pour une commande normale.
    let dossier = match (texte(args, "cwd").filter(|c| !c.trim().is_empty()), root) {
        (Some(c), _) => Some(c.to_string()),
        (None, true) => home_utilisateur(&distro).await,
        (None, false) => None,
    };
    let delai = args["timeout_s"]
        .as_u64()
        .unwrap_or(DELAI_DEFAUT_S)
        .clamp(1, DELAI_MAX_S);
    let avec_mdp = texte(args, "sudo_password").is_some_and(|p| !p.is_empty());
    // Le pseudo-terminal (le même que wsl_start) est ce qui rend les
    // invitations lisibles et répondables ; la commande reste bornée par
    // `timeout -k 5` côté WSL, comme avant. Le mot de passe sudo, mis par
    // Locaryn à la place de {{secret:…}}, part par l'entrée standard du
    // pseudo-terminal — jamais dans la ligne de commande.
    let id = crate::sessions::lancer(
        commande,
        distro.clone(),
        dossier.as_deref(),
        root,
        texte(args, "sudo_password")
            .filter(|p| !p.is_empty())
            .map(str::to_string),
        delai,
    )
    .await?;
    let (r, fin) = crate::sessions::attendre_avec(
        &id,
        Duration::from_secs(delai) + MARGE_DEMARRAGE,
        Some(&semble_invite),
    )
    .await?;
    let sortie = r["output"].as_str().unwrap_or_default().to_string();
    let mut rep = json!({
        "distro": distro,
        "as_root": root,
        "stdout": &sortie,
        // Le pseudo-terminal mêle stdout et stderr : tout arrive dans
        // `stdout`, comme dans une console.
        "stderr": "",
    });
    if let Some(avis) = r.get("wsl_notice") {
        rep["wsl_notice"] = avis.clone();
    }
    match fin {
        crate::sessions::FinAtteinte::Terminee => {
            rep["exit_code"] = r.get("exit_code").cloned().unwrap_or(Value::Null);
            if rep["exit_code"] == json!(124) {
                rep["timed_out"] = json!(true);
                rep["note"] = json!(format!(
                    "Commande interrompue après {delai} s. Pour un serveur ou une tâche longue : `setsid nohup … > ~/journal.log 2>&1 &` puis lisez le journal."
                ));
            }
        }
        crate::sessions::FinAtteinte::Invite => {
            // La commande attend une réponse : on rend la main tout en la
            // laissant vivre — le modèle y répond par wsl_send au lieu de
            // garder wsl_run bloqué jusqu'au délai.
            rep["running"] = json!(true);
            rep["session"] = json!(id);
            let bas = sortie.to_lowercase();
            let mdp_demande = utilise_sudo(commande)
                && !avec_mdp
                && (bas.contains("password for")
                    || bas.contains("a terminal is required")
                    || bas.contains("a password is required"));
            rep["note"] = json!(if mdp_demande {
                format!("sudo attend un mot de passe. Relancez avec sudo_password = {{{{secret:wsl-sudo-{distro}}}}} (s'il n'est pas encore dans le coffre, ask_secret le fait saisir à l'utilisateur sans que vous le voyiez), demandez wsl_allow_sudo, ou répondez à la session avec wsl_send (input = {{{{secret:wsl-sudo-{distro}}}}}).")
            } else {
                "La commande attend une réponse (question, menu, mot de passe) : répondez avec wsl_send (input, ou keys pour un menu plein écran), relisez la suite avec wsl_read, ou arrêtez-la avec wsl_stop.".to_string()
            });
        }
        crate::sessions::FinAtteinte::Delai => {
            // WSL tue la commande lui-même à `delai` (timeout -k 5) : si l'on
            // est encore ici, c'est wsl.exe qui est gelé. On coupe plutôt que
            // de laisser une session fantôme.
            crate::sessions::stop(&json!({ "session": id })).await.ok();
            return Err(format!(
                "WSL n'a pas répondu en {} s",
                (Duration::from_secs(delai) + MARGE_DEMARRAGE).as_secs()
            ));
        }
    }
    Ok(rep)
}

/// La fin de la sortie (déjà nettoyée) ressemble-t-elle à une invitation à
/// répondre : question, menu, mot de passe ? C'est ce qui rend la main au
/// modèle dans `wsl_run`, au lieu de le laisser bloqué jusqu'au délai. Jugée
/// sur la queue, comme un humain qui regarde ce que la commande vient d'écrire.
pub fn semble_invite(sortie: &str) -> bool {
    let bas = sortie.to_lowercase();
    const MOTIFS: &[&str] = &[
        "[y/n]",
        "(y/n)",
        "y/n]",
        "[yes/no]",
        "[oui/non]",
        "<yes>",
        "[ yes ]",
        "[ok]",
        "[ ok ]",
        "[sudo]",
        "password for",
        "a terminal is required",
        "a password is required",
        "mot de passe",
        "press enter",
        "appuyez sur",
        "voulez-vous",
        "do you want",
        "are you sure",
        "should ",
        "(default",
        "[default",
    ];
    let queue: String = bas
        .chars()
        .rev()
        .take(600)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if MOTIFS.iter().any(|m| queue.contains(m)) {
        return true;
    }
    // Une question écrite en toutes lettres : la dernière ligne non vide se
    // termine par « ? », ou ouvre une saisie (« Nom du réseau : »).
    sortie
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .is_some_and(|l| {
            let l = l.trim_end();
            l.ends_with('?') || l.ends_with(':')
        })
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
    (reste.join("\n"), avis)
}

/// Lire un fichier entier (pour le modifier).
async fn lire_tout(distro: &str, chemin: &str, root: bool) -> Result<String, String> {
    let a = exec_args(distro, None, root, &["cat", "--", chemin]);
    let s = wsl::wsl(&a, None, Duration::from_secs(60)).await?;
    if s.code != Some(0) {
        return Err(format!("Lecture impossible : {}", decode(&s.stderr).trim()));
    }
    if s.stdout.contains(&0) {
        return Err(format!("« {chemin} » est un fichier binaire."));
    }
    String::from_utf8(s.stdout).map_err(|_| format!("« {chemin} » n'est pas du texte UTF-8."))
}

/// Écrire un fichier, dossiers créés au besoin.
async fn ecrire(
    distro: &str,
    chemin: &str,
    contenu: &str,
    ajout: bool,
    root: bool,
) -> Result<(), String> {
    let script = r#"mkdir -p -- "$(dirname -- "$1")" && if [ "$2" = a ]; then cat >> "$1"; else cat > "$1"; fi"#;
    let a = exec_args(
        distro,
        None,
        root,
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
    Ok(())
}

/// Remplacer `old` par `new` : unique, ou partout avec `tout`.
pub fn remplacer(
    contenu: &str,
    old: &str,
    new: &str,
    tout: bool,
) -> Result<(String, usize), String> {
    if old.is_empty() {
        return Err("« old » est vide : donnez le passage exact à remplacer.".into());
    }
    let n = contenu.matches(old).count();
    match (n, tout) {
        (0, _) => Err("Passage introuvable : relisez le fichier (wsl_read_file) et copiez-le exactement, espaces et retours à la ligne compris.".into()),
        (1, _) | (_, true) => Ok((contenu.replace(old, new), n)),
        (n, false) => Err(format!("Le passage apparaît {n} fois : donnez plus de contexte pour qu'il soit unique, ou replace_all=true.")),
    }
}

async fn edit_file(args: &Value) -> Result<Value, String> {
    let chemin = requis(args, "path")?;
    let old = texte(args, "old").ok_or("paramètre « old » manquant")?;
    let new = texte(args, "new").ok_or("paramètre « new » manquant")?;
    let root = en_root(args);
    let distro = wsl::choisir(texte(args, "distro")).await?;
    let contenu = lire_tout(&distro, chemin, root).await?;
    let (nouveau, n) = remplacer(&contenu, old, new, args["replace_all"] == json!(true))?;
    ecrire(&distro, chemin, &nouveau, false, root).await?;
    Ok(json!({ "distro": distro, "path": chemin, "replacements": n }))
}

/// Le chemin désigne-t-il un dossier qu'on ne supprime jamais d'un coup ?
pub fn racine_protegee(chemin: &str) -> bool {
    let c = chemin.trim().trim_end_matches('/');
    let c = if c.is_empty() { "/" } else { c };
    RACINES_PROTEGEES.contains(&c)
        || c == "~/"
        || c == "$HOME"
        || c.starts_with("/home/") && c.matches('/').count() == 2
}

async fn delete(args: &Value) -> Result<Value, String> {
    let chemin = requis(args, "path")?;
    if racine_protegee(chemin) {
        return Err(format!(
            "« {chemin} » est un dossier système ou personnel entier : supprimez plutôt ce qu'il contient, élément par élément."
        ));
    }
    let recursif = args["recursive"] == json!(true);
    let distro = wsl::choisir(texte(args, "distro")).await?;
    let mut prog = vec!["rm"];
    if recursif {
        prog.push("-rf");
    }
    prog.push("--");
    prog.push(chemin);
    let a = exec_args(&distro, None, en_root(args), &prog);
    let s = wsl::wsl(&a, None, Duration::from_secs(120)).await?;
    if s.code != Some(0) {
        let e = decode(&s.stderr);
        let indice = if e.contains("Is a directory") {
            " (c'est un dossier : recursive=true)"
        } else if e.contains("Permission denied") {
            " (permission refusée : as_root=true)"
        } else {
            ""
        };
        return Err(format!("Suppression impossible : {}{indice}", e.trim()));
    }
    Ok(json!({ "distro": distro, "deleted": chemin, "recursive": recursif }))
}

async fn move_path(args: &Value) -> Result<Value, String> {
    let de = requis(args, "from")?;
    let vers = requis(args, "to")?;
    let distro = wsl::choisir(texte(args, "distro")).await?;
    let script = r#"mkdir -p -- "$(dirname -- "$2")" && mv -- "$1" "$2""#;
    let a = exec_args(
        &distro,
        None,
        en_root(args),
        &["sh", "-c", script, "locaryn", de, vers],
    );
    let s = wsl::wsl(&a, None, Duration::from_secs(120)).await?;
    if s.code != Some(0) {
        return Err(format!(
            "Déplacement impossible : {}",
            decode(&s.stderr).trim()
        ));
    }
    Ok(json!({ "distro": distro, "from": de, "to": vers }))
}

/// Le dossier personnel de l'utilisateur par défaut de la distribution.
async fn home_utilisateur(distro: &str) -> Option<String> {
    let a = exec_args(distro, None, false, &["sh", "-c", "printf %s \"$HOME\""]);
    let s = wsl::wsl(&a, None, Duration::from_secs(60)).await.ok()?;
    let h = decode(&s.stdout).trim().to_string();
    (s.code == Some(0) && h.starts_with('/')).then_some(h)
}

/// L'entrée sudoers qui autorise `utilisateur` sans mot de passe.
pub fn regle_sudoers(utilisateur: &str) -> Option<String> {
    let valide = !utilisateur.is_empty()
        && utilisateur != "root"
        && utilisateur
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    valide.then(|| format!("{utilisateur} ALL=(ALL) NOPASSWD:ALL\n"))
}

async fn allow_sudo(args: &Value) -> Result<Value, String> {
    let distro = wsl::choisir(texte(args, "distro")).await?;
    let a = exec_args(&distro, None, false, &["id", "-un"]);
    let s = wsl::wsl(&a, None, Duration::from_secs(60)).await?;
    let utilisateur = decode(&s.stdout).trim().to_string();
    let regle = regle_sudoers(&utilisateur)
        .ok_or_else(|| format!("Utilisateur inattendu : « {utilisateur} »."))?;
    if args["confirmed"] != json!(true) {
        return Ok(json!({
            "needs_confirmation": true,
            "distro": distro,
            "user": utilisateur,
            "next": format!("Demandez à l'utilisateur s'il autorise sudo sans mot de passe pour « {utilisateur} » dans {distro} ; s'il accepte, rappelez avec confirmed=true."),
        }));
    }
    // Écrit à côté, vérifié par visudo, puis mis en place : une règle
    // invalide ne doit jamais casser sudo.
    let script = r#"set -e; t=$(mktemp); cat > "$t"; visudo -cf "$t" >/dev/null; install -m 0440 -o root -g root "$t" /etc/sudoers.d/90-locaryn; rm -f "$t""#;
    let a = exec_args(&distro, None, true, &["sh", "-c", script]);
    let s = wsl::wsl(&a, Some(regle.as_bytes()), Duration::from_secs(60)).await?;
    if s.code != Some(0) {
        return Err(format!(
            "Autorisation impossible : {}",
            decode(&s.stderr).trim()
        ));
    }
    Ok(json!({
        "distro": distro,
        "user": utilisateur,
        "sudo_without_password": true,
        "undo": "sudo rm /etc/sudoers.d/90-locaryn",
    }))
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
        en_root(args),
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
    ecrire(&distro, chemin, contenu, ajout, en_root(args)).await?;
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
    #[test]
    fn le_mot_de_passe_sudo_passe_par_l_entree_et_jamais_par_la_commande() {
        let c = super::prefixe_sudo("sudo apt update");
        assert_eq!(c, "sudo -S -p '' -v >/dev/null 2>&1; sudo apt update");
        assert!(!c.contains("{{secret"));
    }

    use super::*;

    #[test]
    fn seules_les_commandes_catastrophiques_sont_bloquees() {
        assert!(catastrophique("sudo rm -rf / --no-preserve-root").is_some());
        assert!(catastrophique("rm -rf /*").is_some());
        assert!(catastrophique("mkfs.ext4 /dev/sdb1").is_some());
        assert!(catastrophique("dd if=x.img of=/dev/sda").is_some());
        assert!(catastrophique("rm -rf build").is_none());
        assert!(catastrophique("sudo apt-get remove nginx").is_none());
        assert!(catastrophique("git reset --hard HEAD~1").is_none());
        assert!(catastrophique("docker compose down -v").is_none());
    }

    #[test]
    fn sudo_est_repere_en_debut_de_mot() {
        assert!(utilise_sudo("sudo apt install -y docker.io"));
        assert!(utilise_sudo("cd x && sudo systemctl restart docker"));
        assert!(!utilise_sudo("echo pseudo"));
        assert!(!utilise_sudo("ls ~/sudoers-backup"));
    }

    #[test]
    fn la_regle_sudoers_ne_vise_qu_un_utilisateur_valide() {
        assert_eq!(
            regle_sudoers("tealo").as_deref(),
            Some("tealo ALL=(ALL) NOPASSWD:ALL\n")
        );
        assert!(regle_sudoers("root").is_none());
        assert!(regle_sudoers("a b").is_none());
        assert!(regle_sudoers("x;rm").is_none());
        assert!(regle_sudoers("").is_none());
    }

    #[test]
    fn une_modification_vise_un_passage_unique() {
        let (r, n) = remplacer("a = 1\nb = 2\n", "b = 2", "b = 3", false).unwrap();
        assert_eq!((r.as_str(), n), ("a = 1\nb = 3\n", 1));
        assert!(remplacer("x x", "x", "y", false).is_err());
        assert_eq!(remplacer("x x", "x", "y", true).unwrap().0, "y y");
        assert!(remplacer("abc", "z", "y", false).is_err());
    }

    #[test]
    fn les_dossiers_entiers_ne_se_suppriment_pas() {
        assert!(racine_protegee("/"));
        assert!(racine_protegee("/etc/"));
        assert!(racine_protegee("/home/tealo"));
        assert!(racine_protegee("~"));
        assert!(!racine_protegee("/home/tealo/TP-Docker"));
        assert!(!racine_protegee("~/projet/build"));
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
        let (reste, avis) =
            separer_avis_wsl("wsl: Failed to start the systemd user session for 'a'.\nerreur: x");
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

    #[test]
    fn une_invite_rend_la_main_au_modele() {
        // La question debconf de tshark, telle qu'un terminal la montre.
        assert!(semble_invite(
            "│ Should non-superusers be able to capture packets?\n│\n│  <Yes>                       <No>\n"
        ));
        assert!(semble_invite("Do you want to continue? [Y/n] "));
        assert!(semble_invite("Voulez-vous continuer ? [O/n]"));
        assert!(semble_invite(
            "Configuration de tshark\n[sudo] password for tealo : "
        ));
        assert!(semble_invite("Répertoire d'installation : "));
        assert!(semble_invite("Press ENTER to continue"));
        assert!(semble_invite("  [ Ok ]   [ Cancel ]"));
        // Un dossier de travail, un journal : rien à répondre.
        assert!(!semble_invite(
            "total 12\ndrwxr-xr-x 2 tealo 4096 Oct 18 10:00 src\n"
        ));
        assert!(!semble_invite("Get:1 http://archive.ubuntu.com/ubuntu jammy/main amd64 [100 kB]\nFetched 100 kB in 1s\n"));
        assert!(!semble_invite(
            "Compiling morph v0.1.0\nwarning: unused variable\n"
        ));
        // C'est la fin qui compte : une question enterrée loin sous la suite
        // ne doit pas faire rendre la main.
        let loin = format!("Continue ? [Y/n]\n{}\n", "ligne de journal\n".repeat(60));
        assert!(!semble_invite(&loin));
    }

    #[test]
    fn les_outils_de_lecture_sont_annonces_comme_tels() {
        let l = tools_list();
        let outil = |n: &str| {
            l["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == n)
                .cloned()
                .unwrap()
        };
        assert_eq!(outil("wsl_read_file")["annotations"]["readOnlyHint"], true);
        assert_eq!(outil("wsl_delete")["annotations"]["destructiveHint"], true);
        assert!(outil("wsl_run").get("annotations").is_none());
    }
}
