//! Hand-owned grammar-kind policy. `class_of` is the loudness gate (exhaustive
//! match, no wildcard); a new `RawKind` makes it non-exhaustive → `cargo check`
//! fails → the kind must be triaged.

pub mod kind_policy;

pub use kind_policy::{Class, class_of};

#[cfg(test)]
mod tests {
    use super::{Class, class_of};
    use crate::raw::RawKind;

    #[test]
    fn class_of_samples() {
        assert_eq!(class_of(RawKind::Procedure), Class::Structural);
        assert_eq!(class_of(RawKind::StatementBlock), Class::Structural);
        assert_eq!(class_of(RawKind::BeginKeyword), Class::Trivia);
        assert_eq!(class_of(RawKind::Comment), Class::Trivia);
        assert_eq!(class_of(RawKind::Error), Class::Recovery);
    }
}
