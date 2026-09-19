#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum TaskOwner {
    Process,
    Plugin(String),
    Surface(String),
}
