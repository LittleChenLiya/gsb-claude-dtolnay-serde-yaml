// Raw pointer field accesses in this module predate the
// `dangerous_implicit_autorefs` lint; the unsafe code is sound as written.
#![allow(dangerous_implicit_autorefs)]

mod cstr;
pub mod emitter;
pub mod error;
pub mod parser;
pub mod tag;
mod util;

use self::error::Error;
