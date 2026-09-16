//! The `#[widget]` attribute — the mechanical half of a `Widget` impl,
//! written for you.
//!
//! # The defect this exists for
//!
//! A composed widget in this workspace was four items long before it said
//! anything:
//!
//! ```ignore
//! #[derive(Debug)]
//! struct Counter { count: Signal<i32> }
//!
//! impl Widget for Counter {
//!     fn debug_name(&self) -> &'static str { "Counter" }      // the type's name
//!     fn kind(&self) -> WidgetKind<'_> { WidgetKind::Composed } // always this
//!     fn key(&self) -> Option<&Key> { None }                    // almost always this
//!     fn build(&self, ctx: &BuildContext) -> WidgetNode { .. }  // the only real one
//! }
//!
//! widget_node_from!(Counter);
//! ```
//!
//! Three of those five lines are derivable from the type, and the fourth —
//! `widget_node_from!` — is a separate macro call that a reader has to
//! remember exists and that fails, when forgotten, as a confusing
//! `Into<WidgetNode>` trait error at the *call site* rather than at the
//! definition. Comparable designs make this a single method or a single
//! function. The gap is not conceptual, it is typing, and typing that a
//! developer does several hundred times in an application is worth removing.
//!
//! # What it looks like instead
//!
//! ```ignore
//! #[derive(Debug)]
//! #[vieww_widget::widget]
//! impl Counter {
//!     fn build(&self, ctx: &BuildContext) -> impl Into<WidgetNode> {
//!         Flex::row().children(children![Text::new(self.count.get().to_string())])
//!     }
//! }
//! ```
//!
//! # Why the attribute goes on the `impl` block, not the struct
//!
//! `build` is a *method*, and an attribute on the struct cannot see one. Two
//! alternatives were considered and rejected:
//!
//! - **A derive on the struct plus a separate `impl Widget`.** That removes
//!   nothing: the `impl Widget` block, with its three mechanical methods, is
//!   exactly what the boilerplate *is*.
//! - **A whole-widget macro taking a function body** (`widget! { fn Counter(..) }`).
//!   That invents a second syntax for declaring a type, which puts every
//!   field's visibility, generics, lifetimes and derives inside a macro this
//!   crate would then have to re-implement. `rustfmt` also does not format
//!   macro bodies, so an application's widgets would stop being formatted.
//!
//! The `impl` block is the smallest thing that already contains `build` and
//! already knows the type's name.
//!
//! # What it generates, and what it deliberately does not
//!
//! From an `impl Type { .. }` block it emits:
//!
//! 1. `impl Widget for Type` carrying every method in the block whose name is
//!    a `Widget` trait method — so `create_state`, `debug_properties`,
//!    `catches_panics` and the rest are written exactly as they always were,
//!    in the same block, and simply land in the right impl.
//! 2. `debug_name` returning the type's own name, unless the block wrote one.
//! 3. `kind` returning `WidgetKind::Composed` (not a link: this is a
//!    proc-macro crate, so `vieww-widget` depends on *it* and cannot be a
//!    dependency in return), unless the block wrote one —
//!    which is how a `Render` or `Inherited` widget opts out without opting
//!    out of the attribute.
//! 4. `From<Type> for WidgetNode`, the `widget_node_from!` call, unless
//!    `#[widget(no_into)]` says the type already has one.
//! 5. An inherent `impl Type` block carrying every method that was *not* a
//!    trait method, so helper methods and constructors keep working.
//!
//! It does **not** add `#[derive(Debug)]`. `Widget: Any + Debug`, so a widget
//! needs it — but an attribute macro that silently derives traits makes the
//! set of traits a type implements invisible at the type's definition, and
//! this workspace's scaffold comment already names a missing `Debug` as the
//! first thing to check when a generated project fails to build. A missing
//! `Debug` fails with the compiler naming `Debug`, which is the right error.
//!
//! # Why the generated code names `Widget` unqualified
//!
//! Every other attribute macro in this ecosystem emits fully-qualified paths
//! — `::serde::Serialize`, and this workspace's own `::vieww_plugin::…` — so
//! that expansion cannot be broken by what the call site happens to have
//! imported. This one deliberately does not, and the reason is the facade.
//!
//! An application depends on `vieww`, not on `vieww-widget`. `vieww`
//! re-exports the widget layer, but a re-export does not put `vieww_widget`
//! into that crate's *extern prelude* — so `::vieww_widget::Widget` in
//! expanded code fails to resolve in exactly the crates this attribute is for,
//! and the error names a crate the developer never wrote down. The usual
//! escape (resolving the dependency's real name out of `Cargo.toml` at expansion
//! time) cannot help either, because the name it would find is `vieww`, and
//! `vieww` is not what `vieww-widget`'s own catalogue is compiled against.
//!
//! So the expansion names `Widget`, `WidgetKind`, `WidgetNode` and
//! `BuildContext` unqualified, and resolves them wherever the widget is
//! written. This costs nothing in practice: those four names are already
//! required at the call site to *write* `build`'s signature, and
//! `use vieww::prelude::*` (or `vieww_widget::prelude::*`) supplies all four.
//! A missing import fails with the compiler naming the type that is missing,
//! which is the error a developer can act on.
//!
//! # `build`'s return type
//!
//! The trait requires `WidgetNode`. The attribute lets `build` return anything
//! that converts into one — `impl Into<WidgetNode>` in the common case, so the
//! trailing `.into()` on every widget's last expression goes away. It does
//! this by keeping the written method, under a reserved name, in the inherent
//! impl and having the trait's `build` call it and convert. A `build` already
//! returning `WidgetNode` goes through core's reflexive `From<T> for T` and
//! costs nothing.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Ident, ImplItem, ItemImpl, Meta, Token};

/// The `Widget` trait methods this attribute recognises and moves into the
/// trait impl.
///
/// Written out rather than "anything with `&self`" because a helper method
/// that happens to share a name with a future trait method should be a
/// compile error at the point the name collides, not silently relocated into
/// a trait impl where its signature no longer matches.
const TRAIT_METHODS: &[&str] = &[
    "debug_name",
    "kind",
    "key",
    "build",
    "debug_properties",
    "catches_panics",
    "create_state",
    "publish_inherited",
    "publish_into",
    "publishes",
    "same_configuration",
];

/// See the crate documentation.
///
/// # Panics
///
/// Emits a compile error — not a panic reaching the user — when applied to
/// anything but an inherent `impl` block, when the block implements a trait,
/// or when it is given an argument other than `no_into`.
#[proc_macro_attribute]
pub fn widget(args: TokenStream, input: TokenStream) -> TokenStream {
    let block = parse_macro_input!(input as ItemImpl);
    let options = match Options::parse(args) {
        Ok(options) => options,
        Err(error) => return error.to_compile_error().into(),
    };

    if let Some((_, path, _)) = &block.trait_ {
        return syn::Error::new_spanned(
            path,
            "#[widget] goes on an inherent `impl Type { .. }` block; this one already \
             implements a trait, so there is nothing for the attribute to write",
        )
        .to_compile_error()
        .into();
    }

    let self_ty = &block.self_ty;
    let (impl_generics, _, where_clause) = block.generics.split_for_impl();

    // Split what was written into the two impls it belongs in.
    let mut trait_items: Vec<ImplItem> = Vec::new();
    let mut inherent_items: Vec<ImplItem> = Vec::new();
    let mut wrote = Written::default();

    for item in block.items.clone() {
        let ImplItem::Fn(function) = &item else {
            inherent_items.push(item);
            continue;
        };
        let name = function.sig.ident.to_string();
        if !TRAIT_METHODS.contains(&name.as_str()) {
            inherent_items.push(item);
            continue;
        }
        match name.as_str() {
            "debug_name" => wrote.debug_name = true,
            "kind" => wrote.kind = true,
            "build" => {
                wrote.build = true;
                // `build` is the one method whose written signature is allowed
                // to differ from the trait's, so it stays inherent under a
                // reserved name and the trait method calls it. See the crate
                // doc's "`build`'s return type".
                let mut inner = function.clone();
                inner.sig.ident = format_ident!("__vieww_widget_build");
                inner.vis = syn::Visibility::Inherited;
                inherent_items.push(ImplItem::Fn(inner));
                continue;
            }
            _ => {}
        }
        trait_items.push(item);
    }

    if !wrote.build && !wrote.kind {
        return syn::Error::new_spanned(
            self_ty,
            "#[widget] found no `build` method. A composed widget needs one; a widget \
             that is not composed needs a `kind` method saying so",
        )
        .to_compile_error()
        .into();
    }

    // The type's own name, for `debug_name`. `impl Counter` gives `Counter`;
    // anything more elaborate (`impl Wrapper<T>`, a path, a reference) has no
    // single obvious name, so the attribute asks for one rather than guessing.
    let debug_name = if wrote.debug_name {
        None
    } else {
        match last_path_segment(self_ty) {
            Some(name) => {
                let literal = name.to_string();
                Some(quote! {
                    fn debug_name(&self) -> &'static str { #literal }
                })
            }
            None => {
                return syn::Error::new_spanned(
                    self_ty,
                    "#[widget] cannot read a name from this type; write \
                     `fn debug_name(&self) -> &'static str` in the block",
                )
                .to_compile_error()
                .into();
            }
        }
    };

    let kind = (!wrote.kind).then(|| {
        quote! {
            fn kind(&self) -> WidgetKind<'_> {
                WidgetKind::Composed
            }
        }
    });

    let build = wrote.build.then(|| {
        quote! {
            fn build(&self, ctx: &BuildContext) -> WidgetNode {
                ::core::convert::Into::into(Self::__vieww_widget_build(self, ctx))
            }
        }
    });

    // The `widget_node_from!` call, folded in. Generic types are skipped:
    // `impl<T> From<Wrapper<T>> for WidgetNode` needs `T`'s own bounds to
    // satisfy `Widget`, which this attribute cannot infer, and a wrong bound
    // is a worse error than a missing impl.
    let into_node = (!options.no_into && block.generics.params.is_empty()).then(|| {
        quote! {
            impl ::core::convert::From<#self_ty> for WidgetNode {
                fn from(widget: #self_ty) -> Self {
                    WidgetNode::new(widget)
                }
            }
        }
    });

    let attrs = &block.attrs;
    let inherent = (!inherent_items.is_empty()).then(|| {
        quote! {
            #(#attrs)*
            impl #impl_generics #self_ty #where_clause {
                #(#inherent_items)*
            }
        }
    });

    quote! {
        #inherent

        impl #impl_generics Widget for #self_ty #where_clause {
            #debug_name
            #kind
            #build
            #(#trait_items)*
        }

        #into_node
    }
    .into()
}

/// Which mechanical methods the block wrote for itself, and so which ones the
/// attribute must not write again.
#[derive(Default)]
struct Written {
    debug_name: bool,
    kind: bool,
    build: bool,
}

/// `#[widget(..)]`'s arguments.
struct Options {
    /// Suppress the generated `From<Type> for WidgetNode`.
    ///
    /// For a type that already has one — `Text` and `Container` and the rest
    /// of this crate's own catalogue reach `widget_node_from!` in a batch at
    /// the bottom of their module, and two impls of one trait is E0119.
    no_into: bool,
}

impl Options {
    fn parse(args: TokenStream) -> Result<Self, syn::Error> {
        let mut options = Self { no_into: false };
        if args.is_empty() {
            return Ok(options);
        }
        let args = proc_macro2::TokenStream::from(args);
        let metas = syn::parse::Parser::parse2(
            Punctuated::<Meta, Token![,]>::parse_terminated,
            args.clone(),
        )?;
        for meta in metas {
            if meta.path().is_ident("no_into") {
                options.no_into = true;
            } else {
                return Err(syn::Error::new_spanned(
                    meta,
                    "unknown #[widget] argument; the only one is `no_into`",
                ));
            }
        }
        Ok(options)
    }
}

/// `Counter` out of `Counter`, `some::path::Counter` or `Counter<'a>`.
fn last_path_segment(ty: &syn::Type) -> Option<Ident> {
    match ty {
        syn::Type::Path(path) => path.path.segments.last().map(|s| s.ident.clone()),
        _ => None,
    }
}
