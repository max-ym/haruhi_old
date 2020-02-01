use std::path::Path;

/// List of routes that are grouped and can be enabled/disabled all at once.
pub trait RouteMatchGroup {

    type Proc;

    fn route(&self) -> &Path;

    fn new_handle(&self) -> Self::Proc;
}