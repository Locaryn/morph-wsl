//! Serveur MCP stdio du morph WSL : le modèle travaille dans les
//! distributions Linux de l'utilisateur (commandes, fichiers, chemins).

mod mcp;
mod sessions;
mod tools;
mod wsl;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    mcp::serve(
        "morph-wsl",
        VERSION,
        tools::tools_list(),
        |name, args| async move { tools::call(&name, args).await },
    )
    .await;
}
