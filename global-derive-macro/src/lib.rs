use proc_macro::TokenStream;
use quote::quote;
use syn::{parse, DeriveInput};

#[proc_macro_derive(ServerGlobal)]
pub fn server_global(input: TokenStream) -> TokenStream {
    let derive: DeriveInput = parse(input).unwrap();
    let ident = derive.ident;
    quote! {
        impl ::wl_wayland::server::Global for #ident {
            fn construct(&self) -> ::wl::server::Resident<dyn ::std::any::Any> {
                ::wl::server::Client::reserve(::std::clone::Clone::clone(self))
            }
            fn on_bind_fn(&self) -> fn(Lease<dyn ::std::any::Any>, &mut ::wl::server::Client, &mut ::wl::server::Lease<::wl_wayland::server::WlDisplay>, &mut ::wl::server::Lease<::wl_wayland::server::WlRegistry>) -> ::wl::server::Result<()> {
                <Self as ::wl_wayland::server::OnBind>::bind
            }
        }
    }.into()
}