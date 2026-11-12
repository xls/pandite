//! Print every file extension the default registry decodes, one per line,
//! lowercase and sorted. The packaging checks compare this list with the
//! file associations of the installer.

fn main() {
    let registry = pandite_codecs::default_registry();
    for ext in registry.decodable_extensions() {
        println!("{ext}");
    }
}
