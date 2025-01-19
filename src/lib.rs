//! derive macro for implementing Display and Error on enums
//!
//! ```rust
//! #[derive(Debug, foxerror::FoxError)]
//! enum Error {
//!     NamedFields { a: i32, b: i32 },
//!     #[err(msg = "a custom message")]
//!     WithMessage(String),
//!     /// or the first line of the doc comment
//!     DocWorksToo,
//! }
//! ```

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    DeriveInput, Token,
};

struct ParsedErrors {
    ident: syn::Ident,
    generics: syn::Generics,
    variants: Vec<Variant>,
}

struct Variant {
    ident: syn::Ident,
    fields: syn::Fields,
    msg: Option<String>,
}

struct AttrArg {
    ident: syn::Ident,
    value: Option<syn::Expr>,
}

impl Parse for AttrArg {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident = input.parse()?;
        let value = if input.parse::<Token![=]>().is_ok() {
            input.parse::<syn::Expr>().ok()
        } else {
            None
        };
        Ok(AttrArg { ident, value })
    }
}

struct AttrArgs(Vec<AttrArg>);

impl Parse for AttrArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = vec![];
        loop {
            args.push(input.parse()?);
            if input.parse::<Token![,]>().is_err() {
                return Ok(Self(args));
            }
        }
    }
}

fn parse_attr_doc(a: &syn::Attribute) -> Option<&syn::Expr> {
    if !matches!(a.style, syn::AttrStyle::Outer) {
        return None;
    }
    let syn::Meta::NameValue(ref nameval) = a.meta else {
        return None;
    };
    if !nameval.path.is_ident("doc") {
        return None;
    }
    Some(&nameval.value)
}

fn parse_attr(a: &syn::Attribute) -> Option<AttrArgs> {
    if !matches!(a.style, syn::AttrStyle::Outer) {
        return None;
    }
    let syn::Meta::List(ref list) = a.meta else {
        return None;
    };
    if !matches!(list.delimiter, syn::MacroDelimiter::Paren(_)) {
        return None;
    }
    if !list.path.is_ident("err") {
        return None;
    }
    Some(list.parse_args().expect("could not parse attr args"))
}

fn expr_str(a: &syn::Expr) -> Option<String> {
    match a {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(s),
            ..
        }) => Some(s.value()),
        _ => None,
    }
    .map(|s| s.strip_prefix(' ').unwrap_or(&s).to_string())
}

fn parse_variant(v: syn::Variant) -> Variant {
    let doc = v.attrs.iter().flat_map(parse_attr_doc).next();
    let args = v.attrs.iter().flat_map(parse_attr).last();
    let amsg = args.and_then(|a| {
        a.0.into_iter()
            .find(|a| a.ident == "msg")
            .and_then(|a| a.value)
    });
    let msg = amsg.as_ref().or(doc).and_then(expr_str);
    Variant {
        ident: v.ident,
        fields: v.fields,
        msg,
    }
}

fn parse_derive(ast: DeriveInput) -> ParsedErrors {
    let ident = ast.ident;
    let generics = ast.generics;
    let syn::Data::Enum(body) = ast.data else {
        panic!("only enums are supported")
    };
    let variants = body.variants.into_iter().map(parse_variant).collect();

    ParsedErrors {
        ident,
        generics,
        variants,
    }
}

fn generate(parsed: ParsedErrors) -> TokenStream {
    let ParsedErrors {
        ident,
        generics,
        variants,
    } = parsed;

    let arms = variants.into_iter().map(|v| {
        let Variant {
            ident: name,
            fields,
            msg,
        } = v;
        let msg = if let Some(msg) = msg {
            quote!(#msg)
        } else {
            let name = name.to_string();
            quote!(#name)
        };
        let mut set = quote!();
        let mut get = vec![];
        let mut fmt = vec![quote!("{}")];

        match fields {
            syn::Fields::Named(fields) => {
                fmt.push(quote!(":"));
                let mut ids = vec![];
                for (fnum, field) in fields.named.into_iter().enumerate() {
                    let fid = syn::Ident::new(format!("arg_{}", fnum).as_ref(), Span::call_site());
                    get.push(quote!(#fid));
                    let fnm = field.ident.expect("missing ident");
                    ids.push(quote!(#fnm));
                    if fnum > 0 {
                        fmt.push(quote!(","));
                    }
                    let fo = format!(" {}: {{}}", fnm);
                    fmt.push(quote!(#fo));
                }
                if !get.is_empty() {
                    set = quote!({#(#ids: #get),*});
                }
            }
            syn::Fields::Unnamed(fields) => {
                fmt.push(quote!(":"));
                for fnum in 0..fields.unnamed.len() {
                    let fid = syn::Ident::new(format!("arg_{}", fnum).as_ref(), Span::call_site());
                    get.push(quote!(#fid));
                    if fnum > 0 {
                        fmt.push(quote!(","));
                    }
                    fmt.push(quote!(" {}"));
                }
                if !get.is_empty() {
                    set = quote!((#(#get),*));
                }
            }
            syn::Fields::Unit => (),
        };

        quote! {
            #ident::#name #set => write!(f, concat!(#(#fmt),*), #msg, #(#get),*)
        }
    });

    quote! {
        #[automatically_derived]
        impl #generics ::core::fmt::Display for #ident #generics {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                match self {
                    #(#arms,)*
                }
            }
        }

        #[automatically_derived]
        impl #generics ::core::error::Error for #ident #generics {}
    }
}

/// the derive macro itself
///
/// ```rust
/// #[derive(Debug, foxerror::FoxError)]
/// enum Error<'a> {
///     /// i am a doc comment
///     /// other lines get ignored
///     NoFields,
///     /// or override the message with an attribute
///     #[err(msg = "i have one field")]
///     OneField(&'a str),
///     /// my favorite numbers are
///     ManyFields(i8, i8, i8, i8),
///     // defaults to the variant name when no doc nor attr
///     NamedFields {
///         species: &'a str,
///         leggies: u64,
///     },
/// }
///
/// assert_eq!(format!("{}", Error::NoFields), "i am a doc comment");
/// assert_eq!(
///     format!("{}", Error::OneField("hello")),
///     "i have one field: hello",
/// );
/// assert_eq!(
///     format!("{}", Error::ManyFields(3, 6, 2, 1)),
///     "my favorite numbers are: 3, 6, 2, 1",
/// );
/// assert_eq!(
///     format!("{}", Error::NamedFields { species: "fox", leggies: 4 }),
///     "NamedFields: species: fox, leggies: 4",
/// );
/// ```
#[proc_macro_derive(FoxError, attributes(err))]
pub fn foxerror(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse(input).unwrap();
    let parsed = parse_derive(input);
    let output = generate(parsed);

    output.into()
}
