#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectionPolicy {
    FirstComplete,
    UniqueMatch,
    UniqueClosure,
    CollectAll,
    SelectApplicable,
}
