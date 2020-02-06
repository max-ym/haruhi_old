extern crate haruhi_macro;
#[macro_use]
extern crate async_trait;
extern crate regex;

pub use haruhi_macro::*;

pub mod route;

pub mod proc;

pub type ResponseCode = u16;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
