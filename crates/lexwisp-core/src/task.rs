#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum TaskOwner {
    Process,
    Invocation(String),
    Surface(String),
}
