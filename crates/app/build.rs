//! En Windows: ícono y datos de versión embebidos en el `.exe`.

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/icons/surco.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
}

#[cfg(windows)]
fn windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../packaging/icons/surco.ico")
        .set("ProductName", "Surco")
        .set("FileDescription", "Surco - reproductor de música");
    if let Err(e) = res.compile() {
        // Sin el ícono la app funciona igual: avisar y seguir.
        println!("cargo:warning=no se pudo embeber el ícono de Windows: {e}");
    }
}

#[cfg(not(windows))]
fn windows_resources() {
    // Compilación cruzada hacia Windows: sin recursos.
}
