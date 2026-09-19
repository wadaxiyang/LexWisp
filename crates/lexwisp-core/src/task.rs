#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum TaskOwner {
    Process,
    Plugin(String),
    Invocation(String),
    Surface(String),
}
