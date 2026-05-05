/// Generic struct that depict a period with a start and end event
/// Mainly used to depicts transaction lifetime
use crate::time;
pub struct Span<T> {
    pub start: T,
    pub end: T,
}

impl Span<time::Tick> {
    pub fn duration(&self) -> time::Tick {
        self.end - self.start
    }
}

impl<T: std::fmt::Display> std::fmt::Display for Span<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{},{}]", self.start, self.end)
    }
}
