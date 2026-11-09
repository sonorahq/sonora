//! Headless accessibility regression suite: each test mounts primitives in a Rune runtime with no
//! window and reads the tree a screen reader would get, asserting each interactive element's
//! role and name. Extend it with every new primitive.

mod a11y_tree {
    pub mod content;
    pub mod elements;
    pub mod overlays;
    pub mod support;
}
