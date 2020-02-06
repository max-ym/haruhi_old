use crate::proc::{RouteHandle, RequestContext};
use regex::Regex;

type Handle = Box<dyn RouteHandle>;

/// List of routes that are grouped and can be enabled/disabled all at once.
pub struct RouteMatchGroup {
    arr: Vec<RouteMatch>,
}

pub struct RouteMatch {
    regex: Regex,
    handle: Handle,
}

impl RouteMatchGroup {

    /// Handle for given
    pub fn handle_for(&self, req: RequestContext) -> Option<&Handle> {
        for i in &self.arr {
            let url = req.path();
            if i.regex.is_match(url.as_ref()) {
                return Some(&i.handle);
            }
        }
        None
    }
}

// TODO: at init of server there always should be defined 'default' handle for unmatched routes
// that will normally just emit '404 not found' error
