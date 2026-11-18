//! Embeds the logo as the icon of `secondbrain.exe` (a Windows resource); other targets need nothing.

const ICON: &str = "../../assets/logo/second-brain.ico";

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed={ICON}");
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        winresource::WindowsResource::new()
            .set_icon(ICON)
            .compile()?;
    }
    Ok(())
}
