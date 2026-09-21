use std::path::PathBuf;

enum ElementType {
    FILE,
    FOLDER
}

pub struct StippingProfile {
    language: String,
    detection: Vec<Detection>,
    strippable: Strippable
}

struct Detection {
    element_type: ElementType,
    element: PathBuf
}

struct Strippable {
    element_type: ElementType,
    element: PathBuf
}