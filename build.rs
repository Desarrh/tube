// Icône et nom de l'application dans l'exe : visibles dans l'Explorateur Windows,
// la barre des tâches et le Gestionnaire des tâches (qui affiche « Tube »).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("FileDescription", "Tube")
            .set("FileVersion", env!("CARGO_PKG_VERSION"))
            .set("ProductVersion", env!("CARGO_PKG_VERSION"))
            .set("Comments", "Client YouTube natif et léger")
            .set("ProductName", "Tube")
            .set("InternalName", "tube")
            .set("OriginalFilename", "Tube.exe")
            .set("CompanyName", "Tube")
            .set("LegalCopyright", "Usage personnel");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icône non intégrée : {e}");
        }
    }
}
