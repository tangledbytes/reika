extern crate proc_macro;

use proc_macro::TokenStream as TS;

use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use syn::{parse_quote, ItemFn, ReturnType, Type};

fn task_run(f: syn::ItemFn) -> Result<TokenStream, TokenStream> {
    if f.sig.asyncness.is_none() {
        let err = syn::Error::new_spanned(&f.sig, "task functions must be async");
        return Err(syn::Error::to_compile_error(&err));
    }
    if !f.sig.generics.params.is_empty() {
        let err = syn::Error::new_spanned(&f.sig, "task functions must not be generic");
        return Err(syn::Error::to_compile_error(&err));
    }
    if f.sig.generics.where_clause.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "task functions must not have `where` clauses");
        return Err(syn::Error::to_compile_error(&err));
    }
    if f.sig.abi.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "task functions must not have an ABI qualifier");
        return Err(syn::Error::to_compile_error(&err));
    }
    if f.sig.variadic.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "task functions must not be variadic");
        return Err(syn::Error::to_compile_error(&err));
    }
    match &f.sig.output {
        ReturnType::Default => {}
        ReturnType::Type(_, ty) => match &**ty {
            Type::Tuple(tuple) if tuple.elems.is_empty() => {}
            Type::Never(_) => {}
            _ => {
                let err = syn::Error::new_spanned(
                    &f.sig,
                    "task functions must either not return a value, return `()` or return `!`",
                );
                return Err(syn::Error::to_compile_error(&err));
            }
        },
    }

    let mut arg_names = Vec::new();
    let mut fargs = f.sig.inputs.clone();

    for arg in fargs.iter_mut() {
        match arg {
            syn::FnArg::Receiver(_) => {
                let err =
                    syn::Error::new_spanned(arg, "task functions must not have receiver arguments");
                return Err(syn::Error::to_compile_error(&err));
            }
            syn::FnArg::Typed(t) => match t.pat.as_mut() {
                syn::Pat::Ident(id) => {
                    arg_names.push(id.ident.clone());
                    id.mutability = None;
                }
                _ => {
                    let err = syn::Error::new_spanned(
                        arg,
                        "pattern matching in task arguments is not yet supported",
                    );
                    return Err(syn::Error::to_compile_error(&err));
                }
            },
        }
    }

    let task_ident = f.sig.ident.clone();
    let task_inner_ident = format_ident!("__{}_task", task_ident);
    let storage_name_str = format!("__STORAGE_{}_task", task_ident);
    let task_storage_ident = Ident::new(&storage_name_str.to_uppercase(), task_ident.span());

    let mut task_inner = f;
    let visibility = task_inner.vis.clone();
    task_inner.vis = syn::Visibility::Inherited;
    task_inner.sig.ident = task_inner_ident.clone();

    let mut task_outer: ItemFn = parse_quote! {
        #visibility fn #task_ident(#fargs) -> ::async_executor::TaskRef {
            type Fut = impl ::core::future::Future + 'static;
            static mut #task_storage_ident: ::async_executor::TaskStorage<Fut> = ::async_executor::TaskStorage::new();
            unsafe { #task_storage_ident.prepare_task(move || #task_inner_ident(#(#arg_names,)*)) }
        }
    };

    task_outer.attrs.append(&mut task_inner.attrs.clone());

    let result = quote! {
        // This is the user's task function, renamed.
        // We put it outside the #task_ident fn below, because otherwise
        // the items defined there (such as STORAGE...) would be in scope
        // in the user's code.
        #[doc(hidden)]
        #task_inner

        #task_outer
    };

    Ok(result)
}

#[proc_macro_attribute]
pub fn task(_: TS, item: TS) -> TS {
    let f = syn::parse_macro_input!(item as syn::ItemFn);

    task_run(f).unwrap_or_else(|x| x).into()
}
