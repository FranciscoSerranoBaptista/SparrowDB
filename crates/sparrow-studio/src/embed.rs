use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../packages/studio/dist/"]
pub struct Assets;
