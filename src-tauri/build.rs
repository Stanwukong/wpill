fn main() {
    // The Google OAuth client is baked in from the environment (never committed); rebuild when it changes.
    println!("cargo:rerun-if-env-changed=CAY_GOOGLE_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=CAY_GOOGLE_CLIENT_SECRET");
    tauri_build::build()
}
