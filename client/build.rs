// Иконка .exe: ресурс Windows добавляем только при сборке под windows (в том числе кросс из Linux).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            panic!("не удалось встроить иконку: {e}");
        }
    }
}
