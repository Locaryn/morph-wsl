# morph-wsl

Morph [Locaryn](https://github.com/Locaryn/Locaryn) : donne au modèle l'accès à vos distributions **WSL** (Ubuntu, Debian, Alpine…), sans ouvrir de terminal.

| Outil | Ce qu'il fait |
| --- | --- |
| `wsl_distros` | Liste les distributions installées, leur état et celle par défaut |
| `wsl_run` | Exécute une commande bash (en tant que l'utilisateur, `as_root` pour root) et rend `exit_code`, `stdout`, `stderr` ; si la commande attend une réponse, rend la main avec un `session` à continuer par `wsl_send` |
| `wsl_read_file` | Lit un fichier texte Linux |
| `wsl_write_file` | Crée ou remplace un fichier (dossiers créés au besoin, `append` possible) |
| `wsl_edit_file` | Remplace un passage exact d'un fichier, sans tout réécrire |
| `wsl_move` | Déplace ou renomme un fichier ou un dossier |
| `wsl_delete` | Supprime un fichier, ou un dossier avec `recursive` |
| `wsl_list_dir` | Liste un dossier, jusqu'à 4 niveaux (`.git`, `node_modules`, `target`… cités sans être parcourus) |
| `wsl_path` | `C:\Users\…` ↔ `/mnt/c/Users/…`, et `/home/…` → `\\wsl.localhost\Distro\home\…` |
| `wsl_allow_sudo` | Autorise sudo sans mot de passe pour l'utilisateur (une fois, sur son accord) |

**Windows 10 (2004) et 11, x64 et ARM64.** Il faut WSL et au moins une distribution (`wsl --install -d Ubuntu`).

## Installer

Dans Locaryn : **Extensions → Découvrir → WSL → Installer**. Le morph démarre avec l'application ; demandez par exemple « compile mon projet ~/api dans Ubuntu et lance les tests ».

## Sécurité

- L'accord de l'utilisateur passe par **Locaryn** (fenêtre d'approbation des outils, selon le niveau de permission choisi). Le morph ne bloque lui-même que les commandes qui détruiraient le système ou un disque (`rm -rf /`, `mkfs`, `dd of=/dev/…`), et `wsl_delete` refuse un dossier système ou personnel entier.
- Chaque commande passe par `wsl.exe --exec` : chemins et contenus sont transmis comme arguments ou sur l'entrée standard, jamais recollés dans une ligne de shell. Seule la commande de `wsl_run` est volontairement confiée à `bash -lc`.
- **sudo** : sans mot de passe possible, il échoue et le modèle propose `wsl_allow_sudo`. Celui-ci n'agit qu'avec `confirmed=true` (après accord explicite) : il pose `/etc/sudoers.d/90-locaryn`, vérifié par `visudo`. Pour l'annuler : `sudo rm /etc/sudoers.d/90-locaryn`.
- Chaque commande est bornée dans le temps (120 s par défaut, 30 min au plus). Une commande qui demande une interaction (question `[Y/n]`, menu, mot de passe) n'attend plus le délai : `wsl_run` rend la main avec un identifiant de session, et le modèle y répond par `wsl_send`.

## Développer

```bash
cargo test
cargo build --release
```

Le serveur MCP (`locaryn-wsl-mcp.exe`) parle JSON-RPC sur stdio, une requête par ligne. Une release se publie en poussant un tag `v*` : la CI construit les paquets `windows-x86_64` et `windows-aarch64`.

## Licence

Apache-2.0
