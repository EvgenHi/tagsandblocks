#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]

use wayland_client;
// import objects from the core protocol if needed
use wayland_client::protocol::*;

// This module hosts a low-level representation of the protocol objects
// you will not need to interact with it yourself, but the code generated
// by the generate_client_code! macro will use it
pub mod __interfaces {
    // import the interfaces from the core protocol if needed
    use wayland_client::protocol::__interfaces::*;
    wayland_scanner::generate_interfaces!("./protocols/river-status-unstable-v1.xml");
}
use self::__interfaces::*;

// This macro generates the actual types that represent the wayland objects of
// your custom protocol
wayland_scanner::generate_client_code!("./protocols/river-status-unstable-v1.xml");
