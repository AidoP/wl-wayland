# Wayland
An implementation of core [Wayland](https://wayland.freedesktop.org/) interfaces and convenience functions for accelerating the development of Wayland clients and servers using the [`wl`](https://github.com/AidoP/wl) crate.

# Example

```rust
use wl::server::prelude::*;
use wl_wayland::server::prelude::*;

fn main() {
    let server = Server::bind().expect("Unable to bind to socket");
    let mut display = WlDisplay::new(vec![]);
    display.register_global(WlCompositor);
    server.start(display, DispatchErrorHandler, drop_handler)
}

#[protocol("protocol/wayland.toml")]
mod protocol {
    use super::WlDisplay as WlDisplay;
    use super::WlRegistry as WlRegistry;
    use super::WlCallback as WlCallback;

    use super::WlShm as WlShm;
    use super::WlShmPool as WlShmPool;
    use super::WlBuffer as WlBuffer;

    type WlCompositor = super::WlCompositor;
}

#[derive(Clone, Global)]
pub struct WlCompositor;
impl OnBind for WlCompositor {}
impl protocol::WlCompositor for Lease<WlCompositor> {
    fn create_surface(&mut self, client: &mut Client, id: NewId) -> Result<()> {
        todo!()
    }
    fn create_region(&mut self, client: &mut Client, id: NewId) -> Result<()> {
        todo!()
    }
}
```