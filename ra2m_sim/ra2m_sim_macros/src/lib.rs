use proc_macro::{Span, TokenStream};
use syn::{parse_macro_input, DeriveInput, Ident};

mod module;
mod trace;
mod types;

// Set of Derive/attributes function for derive(Module) ===========================================
/// Derive Module trait on structure
#[proc_macro_derive(Module, attributes(port))]
pub fn module(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;

    match module::expand(&ast, name) {
        Ok(s) => s.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// Attribute macro for tagging init function
#[proc_macro_attribute]
pub fn init(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(item as syn::ItemFn);
    let fn_name = &ast.sig.ident;

    match module::expand_init(&ast, fn_name) {
        Ok(s) => s.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
// Attribute macro for default init function
#[proc_macro_attribute]
pub fn default_init(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(item as syn::ItemImpl);
    let impl_name = Ident::new(&types::typename(&ast.self_ty), Span::call_site().into());

    match module::expand_default_init(&ast, &impl_name) {
        Ok(s) => s.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// Attribute macro for tagging teardown function
#[proc_macro_attribute]
pub fn teardown(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(item as syn::ItemFn);
    let fn_name = &ast.sig.ident;

    match module::expand_teardown(&ast, fn_name) {
        Ok(s) => s.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// Attribute macro for default teardownfunction
#[proc_macro_attribute]
pub fn default_teardown(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(item as syn::ItemImpl);
    let impl_name = Ident::new(&types::typename(&ast.self_ty), Span::call_site().into());

    match module::expand_default_teardown(&ast, &impl_name) {
        Ok(s) => s.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// Set of Derive/attributes function for derive(Trace) ============================================

#[proc_macro_derive(Trace, attributes(history, trace, trace_custom))]
pub fn trace(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;

    match trace::expand(&ast, name) {
        Ok(s) => s.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// Attribute macro for tagging init function
#[proc_macro_attribute]
pub fn default_history(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let mut ast = parse_macro_input!(input as syn::ItemStruct);
    let name = &ast.ident.clone();

    match trace::expand_default_history(&mut ast, name) {
        Ok(s) => s.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
