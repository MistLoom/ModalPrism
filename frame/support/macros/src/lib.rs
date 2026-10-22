//! `#[client(id = "...", ext = echo)]`：为 `dyn AbilityCtx` 扩展 typed handle。
//!
//! 各能力 crate 声明自己的 client trait；调用方 `use FooExt` 后 `ctx.foo().method(args)`，
//! 等价于 TypeScript 的声明合并 / 模块增强。禁止依赖对方 Provider。

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    FnArg, ItemTrait, LitStr, Result as SynResult, Token, TraitItem, Type, parse_macro_input,
    parse_quote,
};

struct ClientArgs {
    id: LitStr,
    ext: Option<syn::Ident>,
}

impl Parse for ClientArgs {
    fn parse(input: ParseStream) -> SynResult<Self> {
        let mut id = None;
        let mut ext = None;
        let pairs = Punctuated::<syn::Meta, Token![,]>::parse_terminated(input)?;
        for meta in pairs {
            match meta {
                syn::Meta::NameValue(nv) if nv.path.is_ident("id") => {
                    if let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(s),
                        ..
                    }) = nv.value
                    {
                        id = Some(s);
                    } else {
                        return Err(syn::Error::new_spanned(&nv.value, "id 须为字符串"));
                    }
                }
                syn::Meta::NameValue(nv) if nv.path.is_ident("ext") => {
                    if let syn::Expr::Path(p) = nv.value {
                        ext = p.path.get_ident().cloned();
                    } else {
                        return Err(syn::Error::new_spanned(&nv.value, "ext 须为标识符"));
                    }
                }
                other => {
                    return Err(syn::Error::new_spanned(other, "仅支持 id / ext"));
                }
            }
        }
        Ok(Self {
            id: id.ok_or_else(|| syn::Error::new(input.span(), "需要 id = \"...\""))?,
            ext,
        })
    }
}

fn to_snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[proc_macro_attribute]
pub fn client(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand_attr(attr, item, parse_quote!(::ability_support))
}

/// 供 `ability-sdk` 再导出：展开路径走 `::ability_sdk`。
#[proc_macro_attribute]
pub fn sdk_client(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand_attr(attr, item, parse_quote!(::ability_sdk))
}

fn expand_attr(attr: TokenStream, item: TokenStream, krate: syn::Path) -> TokenStream {
    let args = parse_macro_input!(attr as ClientArgs);
    let trait_item = parse_macro_input!(item as ItemTrait);
    match expand(args, trait_item, krate) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(
    args: ClientArgs,
    trait_item: ItemTrait,
    krate: syn::Path,
) -> SynResult<proc_macro2::TokenStream> {
    let mut trait_item = trait_item;
    trait_item
        .attrs
        .push(parse_quote!(#[allow(dead_code, async_fn_in_trait)]));
    let trait_ident = &trait_item.ident;
    let vis = &trait_item.vis;
    let id_lit = &args.id;
    let ext_name = args
        .ext
        .unwrap_or_else(|| format_ident!("{}", to_snake(&trait_ident.to_string())));
    let ext_as = format_ident!("{ext_name}_as");
    let handle = format_ident!("{trait_ident}Handle");
    let ext_trait = format_ident!("{trait_ident}Ext");

    let mut inherent = Vec::new();
    for item in &trait_item.items {
        let TraitItem::Fn(m) = item else {
            return Err(syn::Error::new_spanned(item, "client trait 只允许方法"));
        };
        if m.sig.asyncness.is_none() {
            return Err(syn::Error::new_spanned(&m.sig, "方法须为 async"));
        }
        let method_ident = &m.sig.ident;
        let method_name = method_ident.to_string();
        let mut typed_args = Vec::new();
        for input in &m.sig.inputs {
            match input {
                FnArg::Receiver(_) => {}
                FnArg::Typed(pat) => typed_args.push(pat),
            }
        }
        if typed_args.len() != 1 {
            return Err(syn::Error::new_spanned(
                &m.sig,
                "方法须为 async fn name(&self, args: T) -> Result<U>",
            ));
        }
        let arg = typed_args[0];
        let arg_pat = &arg.pat;
        let arg_ty = &arg.ty;
        let ret_ty: Type = match &m.sig.output {
            syn::ReturnType::Type(_, ty) => (**ty).clone(),
            syn::ReturnType::Default => {
                return Err(syn::Error::new_spanned(&m.sig, "须写返回类型 Result<T>"));
            }
        };
        inherent.push(quote! {
            pub fn #method_ident(self, #arg_pat: #arg_ty) -> impl ::core::future::Future<Output = #ret_ty> + Send + 'a {
                let ctx = self.ctx;
                let key = self.key;
                async move {
                    let method = #krate::MethodId::new(#method_name)
                        .map_err(|e| #krate::Error::invalid_args(e.to_string()))?;
                    let payload = ::serde_json::to_value(#arg_pat)
                        .map_err(|e| #krate::Error::invalid_args(e.to_string()))?;
                    let raw = ctx.call(&key, &method, payload).await?;
                    ::serde_json::from_value(raw)
                        .map_err(|e| #krate::Error::failed(e.to_string()))
                }
            }
        });
    }

    Ok(quote! {
        #trait_item

        #vis struct #handle<'a> {
            ctx: &'a dyn #krate::AbilityCtx,
            key: ::std::string::String,
        }

        impl<'a> #handle<'a> {
            #(#inherent)*
        }

        #vis trait #ext_trait<'a> {
            fn #ext_name(self) -> #handle<'a>;
            fn #ext_as(self, key: &str) -> #handle<'a>;
        }

        impl<'a> #ext_trait<'a> for &'a dyn #krate::AbilityCtx {
            fn #ext_name(self) -> #handle<'a> {
                #handle { ctx: self as &'a dyn #krate::AbilityCtx, key: #id_lit.to_string() }
            }
            fn #ext_as(self, key: &str) -> #handle<'a> {
                #handle { ctx: self as &'a dyn #krate::AbilityCtx, key: key.to_string() }
            }
        }

        impl<'a> #ext_trait<'a> for &'a (dyn #krate::AbilityCtx + Send) {
            fn #ext_name(self) -> #handle<'a> {
                #handle { ctx: self as &'a dyn #krate::AbilityCtx, key: #id_lit.to_string() }
            }
            fn #ext_as(self, key: &str) -> #handle<'a> {
                #handle { ctx: self as &'a dyn #krate::AbilityCtx, key: key.to_string() }
            }
        }

        impl<'a> #ext_trait<'a> for &'a (dyn #krate::AbilityCtx + Sync) {
            fn #ext_name(self) -> #handle<'a> {
                #handle { ctx: self as &'a dyn #krate::AbilityCtx, key: #id_lit.to_string() }
            }
            fn #ext_as(self, key: &str) -> #handle<'a> {
                #handle { ctx: self as &'a dyn #krate::AbilityCtx, key: key.to_string() }
            }
        }

        impl<'a> #ext_trait<'a> for &'a (dyn #krate::AbilityCtx + Send + Sync) {
            fn #ext_name(self) -> #handle<'a> {
                #handle { ctx: self as &'a dyn #krate::AbilityCtx, key: #id_lit.to_string() }
            }
            fn #ext_as(self, key: &str) -> #handle<'a> {
                #handle { ctx: self as &'a dyn #krate::AbilityCtx, key: key.to_string() }
            }
        }
    })
}
