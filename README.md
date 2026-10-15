# morph-wsl

Morph [Locaryn](https://github.com/Locaryn/Locaryn) : donne au modèle l'accès à vos distributions **WSL** (Ubuntu, Debian, Alpine…), sans ouvrir de terminal.

| Outil | Ce qu'il fait |
| --- | --- |
| `wsl_distros` | Liste les distributions installées, leur état et celle par défaut |
| `wsl_run` | Exécute une commande bash et rend `exit_code`, `stdout`, `stderr` |
| `wsl_read_file` | Lit un fichier texte Linux |
| `wsl_write_file` | Écrit (ou complète) un fichier Linux, dossiers créés au besoin |
| `wsl_list_dir` | Liste un dossier, jusqu'à 4 niveaux (`.git`, `node_modules`, `target`… cités sans être parcourus) |
| `wsl_path` | `C:\Users\…` ↔ `/mnt/c/Users/…`, et `/home/…` → `\wsl.localhost\Distro\home\…` |

**Windows 10 (2004) et 11, x64 et ARM64.** Il faut WSL et au moins une distribution (`wsl --install -d Ubuntu`).

## Installer

Dans Locaryn : **Extensions → Découvrir → WSL → Installer**. Le morph démarre avec l'application ; demandez par exemple « compile mon projet ~/api dans Ubuntu et lance les tests ».

## Sécurité

- Chaque commande passe par `wsl.exe --exec` : les chemins et le contenu des fichiers sont transmis comme arguments ou sur l'entrée standard, jamais recollés dans une ligne de shell. Seule la commande de `wsl_run` est volontairement confiée à `bash -lc`.
- Les commandes destructrices (`rm`, `git reset --hard`, `docker system prune`, `apt remove`, `mkfs`, `shutdown`…) et toute exécution en root (`as_root`) reviennent avec `needs_confirmation` : le modèle doit vous demander votre accord avant de rappeler l'outil avec `confirmed=true`.
- `sudo` ne peut pas recevoir de mot de passe : le modèle propose alors `as_root`, soumis à votre accord.
- Chaque commande est bornée dans le temps (120 s par défaut, 30 min au plus) par `timeout` côté Linux, puis côté Windows.

## Développer

```bash
cargo test
cargo build --release
```

Le serveur MCP (`locaryn-wsl-mcp.exe`) parle JSON-RPC sur stdio, une requête par ligne. Une release se publie en poussant un tag `v*` : la CI construit les paquets `windows-x86_64` et `windows-aarch64`.

## Licence

Apache-2.0
