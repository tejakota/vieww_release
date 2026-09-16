//! The `#[vieww_plugin]` registration macro.
//!
//! A plugin author implements `vieww_plugin::abi::Plugin` on some type and
//! needs exactly one more thing before it is a real, loadable `cdylib`: a
//! `#[no_mangle] extern "C" fn vieww_plugin_entry` at a fixed symbol name,
//! built by calling `vieww_plugin::abi::leak_vtable` on a first instance.
//! Writing that by hand is a handful of lines that are the same for every
//! plugin that will ever exist and offer nothing for a plugin author to get
//! right or wrong beyond copying them correctly — exactly the kind of
//! boilerplate a proc macro exists to remove, and the *only* `unsafe extern
//! "C"` wiring `vieww-plugin`'s own module doc promises a plugin author never
//! has to write themselves.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse2, Item};

/// Generates the `#[no_mangle] extern "C" fn vieww_plugin_entry` a host looks
/// up by the fixed symbol name `vieww_plugin::abi::ENTRY_SYMBOL` names.
///
/// # What this expects
///
/// Place this on the struct or enum that implements
/// `vieww_plugin::abi::Plugin`. That type must also implement `Default`: the
/// generated entry point takes no arguments — a `cdylib`'s exported symbols
/// are plain functions, nothing can pass constructor arguments across that
/// boundary — so `Default::default()` is the only way to produce a first
/// instance without this macro inventing a second, ad hoc construction
/// convention and guessing what to name it. A plugin with real
/// construction-time setup does it in `Plugin::init`, which *does* receive
/// arguments (the host's `HostVTable`) — see that method's own doc for why
/// that, not construction, is where a plugin should do anything that can
/// fail.
///
/// This attribute takes no arguments of its own.
///
/// # What it expands to
///
/// The annotated item, unchanged, plus (writing `Type` for whatever name was
/// annotated):
///
/// ```ignore
/// #[no_mangle]
/// pub extern "C" fn vieww_plugin_entry() -> *mut ::vieww_plugin::abi::PluginVTable {
///     ::vieww_plugin::abi::leak_vtable(<Type as ::core::default::Default>::default())
/// }
/// ```
///
/// Fully qualified paths throughout, so the expansion does not depend on
/// which names a plugin crate has (or has not) brought into scope with `use`.
#[proc_macro_attribute]
pub fn vieww_plugin(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand(attr.into(), item.into()).into()
}

/// The actual expansion logic, over `proc_macro2::TokenStream` rather than
/// `proc_macro::TokenStream` so it can be exercised directly by this crate's
/// own unit tests below — `proc_macro`'s types are only usable from inside a
/// real macro invocation, `proc_macro2`'s are not.
fn expand(attr: TokenStream2, item: TokenStream2) -> TokenStream2 {
    if !attr.is_empty() {
        return syn::Error::new_spanned(
            attr,
            "#[vieww_plugin] takes no arguments; implement `Default` on the annotated type instead",
        )
        .to_compile_error();
    }

    let parsed = match parse2::<Item>(item) {
        Ok(item) => item,
        Err(err) => return err.to_compile_error(),
    };

    let ident = match &parsed {
        Item::Struct(item_struct) => &item_struct.ident,
        Item::Enum(item_enum) => &item_enum.ident,
        other => {
            return syn::Error::new_spanned(
                other,
                "#[vieww_plugin] must be placed on a struct or enum that implements \
                 `vieww_plugin::abi::Plugin` and `Default`",
            )
            .to_compile_error();
        }
    };

    quote! {
        #parsed

        #[doc(hidden)]
        #[no_mangle]
        pub extern "C" fn vieww_plugin_entry() -> *mut ::vieww_plugin::abi::PluginVTable {
            ::vieww_plugin::abi::leak_vtable(<#ident as ::core::default::Default>::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_a_struct_into_itself_plus_a_real_entry_point() {
        let expanded = expand(
            TokenStream2::new(),
            quote! {
                #[derive(Default)]
                struct Greeter;
            },
        )
        .to_string();

        assert!(
            expanded.contains("struct Greeter"),
            "the original item must survive unchanged"
        );
        assert!(expanded.contains("no_mangle"));
        assert!(expanded.contains("vieww_plugin_entry"));
        assert!(expanded.contains("leak_vtable"));
        assert!(
            expanded.contains("Greeter"),
            "the entry point must construct this exact type"
        );
    }

    #[test]
    fn accepts_an_enum_too() {
        let expanded = expand(
            TokenStream2::new(),
            quote! {
                #[derive(Default)]
                enum Mode {
                    #[default]
                    Standard,
                    Advanced,
                }
            },
        )
        .to_string();

        assert!(expanded.contains("enum Mode"));
        assert!(expanded.contains("vieww_plugin_entry"));
        assert!(expanded.contains("Mode"));
    }

    #[test]
    fn rejects_attribute_arguments_with_a_compile_error_rather_than_silently_ignoring_them() {
        let expanded = expand(quote! { some, args }, quote! { struct X; }).to_string();
        assert!(expanded.contains("compile_error"));
        assert!(
            !expanded.contains("vieww_plugin_entry"),
            "no entry point should be emitted for a rejected input"
        );
    }

    #[test]
    fn rejects_items_that_are_not_a_struct_or_enum() {
        let expanded = expand(TokenStream2::new(), quote! { fn not_a_type() {} }).to_string();
        assert!(expanded.contains("compile_error"));
        assert!(!expanded.contains("vieww_plugin_entry"));
    }

    #[test]
    fn rejects_unparseable_input_instead_of_panicking() {
        // A bare literal is valid, balanced tokens but not a valid `Item` —
        // exercises the `parse2::<Item>` error path without needing tokens
        // syn's own tokenizer would refuse to accept at all.
        let expanded = expand(TokenStream2::new(), quote! { 42 }).to_string();
        assert!(expanded.contains("compile_error"));
    }
}
