// Icône et métadonnées des exécutables Windows.
fn main() {
    println!("cargo:rerun-if-changed=FaceSort.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("FaceSort.ico");
        res.set("ProductName", "FaceSort");
        res.set("FileDescription", "FaceSort - tri de photos par personne");
        res.set("CompanyName", "Les-Doubles-A");
        res.set("LegalCopyright", "Les-Doubles-A");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Ressources Windows non compilées : {e}");
        }
    }
}
