#![recursion_limit = "512"]

extern crate proc_macro;

#[macro_use]
extern crate synstructure;

use crate::proc_macro::TokenStream;
use quote::{quote, quote_spanned, ToTokens};

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::{
    parse_macro_input, punctuated::Punctuated, token::Paren, Field, Fields, FieldsUnnamed, FnArg,
    Ident, ItemTrait, Path, PathArguments, PathSegment, ReturnType, TraitBound, TraitBoundModifier,
    TraitItem, TraitItemMethod, Type, TypeParamBound, TypeVerbatim, Variant, Visibility,
};

#[derive(Debug)]
struct Procedure {
    arg_types: Vec<Type>,
    mut_receiver: bool,
    ident: Option<Ident>,
    return_type: Option<Type>,
}

fn generate_enum(methods: &[Procedure]) -> Vec<Variant> {
    methods
        .iter()
        .map(|method| Variant {
            ident: method.ident.clone().unwrap(),
            attrs: vec![],
            discriminant: None,
            fields: {
                let mut fields = Punctuated::new();
                for ty in &method.arg_types {
                    fields.push(Field {
                        attrs: vec![],
                        ident: None,
                        ty: ty.clone(),
                        colon_token: None,
                        vis: Visibility::Inherited,
                    });
                }
                fields.push(Field {
                    attrs: vec![],
                    ident: None,
                    ty: Type::Verbatim(TypeVerbatim {
                        tts: quote! {
                            u64
                        },
                    }),
                    colon_token: None,
                    vis: Visibility::Inherited,
                });
                Fields::Unnamed(FieldsUnnamed {
                    paren_token: Paren(Span::call_site()),
                    unnamed: fields,
                })
            },
        })
        .collect::<Vec<_>>()
}

fn generate_return_variants(methods: &[Procedure]) -> Vec<Variant> {
    methods
        .iter()
        .map(|method| Variant {
            ident: method.ident.clone().unwrap(),
            attrs: vec![],
            discriminant: None,
            fields: {
                let mut fields = Punctuated::new();
                let ty = &method.return_type;
                fields.push(Field {
                    attrs: vec![],
                    ident: None,
                    ty: Type::Verbatim(TypeVerbatim {
                        tts: quote! {
                            <#ty as ::vessels::protocol::Value>::Item
                        },
                    }),
                    colon_token: None,
                    vis: Visibility::Inherited,
                });
                fields.push(Field {
                    attrs: vec![],
                    ident: None,
                    ty: Type::Verbatim(TypeVerbatim {
                        tts: quote! {
                            u64
                        },
                    }),
                    colon_token: None,
                    vis: Visibility::Inherited,
                });
                fields.push(Field {
                    attrs: vec![],
                    ident: None,
                    ty: Type::Verbatim(TypeVerbatim {
                        tts: quote! {
                            u64
                        },
                    }),
                    colon_token: None,
                    vis: Visibility::Inherited,
                });
                Fields::Unnamed(FieldsUnnamed {
                    paren_token: Paren(Span::call_site()),
                    unnamed: fields,
                })
            },
        })
        .collect::<Vec<_>>()
}

fn generate_remote_impl(ident: &Ident, methods: &[Procedure]) -> proc_macro2::TokenStream {
    let call_inner = prefix(ident, "Call_Inner");
    let call = prefix(ident, "Call");
    let channel = prefix(ident, "Channel");
    let mut stream = proc_macro2::TokenStream::new();
    for method in methods.iter() {
        let index_ident = method.ident.clone().unwrap();
        let ident = &method.ident;
        let mut arg_stream = proc_macro2::TokenStream::new();
        let mut arg_names_stream = proc_macro2::TokenStream::new();
        if method.mut_receiver {
            arg_stream.extend(quote! {
                &mut self,
            });
        } else {
            arg_stream.extend(quote! {
                &self,
            });
        }
        let mut call_sig = proc_macro2::TokenStream::new();
        for (index, ty) in method.arg_types.iter().enumerate() {
            let ident = Ident::new(&format!("_{}", index), Span::call_site());
            arg_stream.extend(quote! {
                #ident: #ty,
            });
            arg_names_stream.extend(quote! {
                #ident,
            });
        }
        arg_names_stream.extend(quote! {
            _proto_id,
        });
        call_sig.extend(quote! {
            (#arg_names_stream)
        });
        let return_type = &method.return_type;
        stream.extend(quote! {
            fn #ident(#arg_stream) -> #return_type {
                let _proto_id = self.next_id();
                let (ct, ct1) = ::vessels::protocol::Context::new();
                self.channels.write().unwrap().insert(_proto_id, #channel::#ident(Box::new(ct1)));
                self.queue.write().unwrap().push_back(#call {call: #call_inner::#index_ident#call_sig});
                self.task.notify();
                <#return_type as ::vessels::protocol::Value>::construct(ct)
            }
        });
    }
    stream
}

fn generate_serialize_impl(ident: &Ident, methods: &[Procedure]) -> proc_macro2::TokenStream {
    let call_inner = prefix(ident, "Call_Inner");
    let mut arms = proc_macro2::TokenStream::new();
    for (index, method) in methods.iter().enumerate() {
        let ident = &method.ident;
        let mut sig = proc_macro2::TokenStream::new();
        let mut args = proc_macro2::TokenStream::new();
        let mut element_calls = proc_macro2::TokenStream::new();
        let t_len = method.arg_types.len() + 2;
        for index in 0..=method.arg_types.len() {
            let ident = Ident::new(&format!("_{}", index), Span::call_site());
            args.extend(quote! {
                #ident,
            });
            element_calls.extend(quote! {
                seq.serialize_element(#ident)?;
            });
        }
        sig.extend(quote! {
            (#args)
        });
        arms.extend(quote! {
            #call_inner::#ident#sig => {
                let mut seq = serializer.serialize_seq(Some(#t_len))?;
                seq.serialize_element(&#index)?;
                #element_calls
                seq.end()
            },
        });
    }
    arms
}

fn generate_serialize_return_impl(
    ident: &Ident,
    methods: &[Procedure],
) -> proc_macro2::TokenStream {
    let response = prefix(ident, "Response");
    let mut arms = proc_macro2::TokenStream::new();
    for method in methods {
        let ident = &method.ident;
        arms.extend(quote! {
            #response::#ident(data, idx, m) => {
                let mut seq = serializer.serialize_seq(Some(3))?;
                seq.serialize_element(m)?;
                seq.serialize_element(idx)?;
                seq.serialize_element(data)?;
                seq.end()
            },
        });
    }
    arms
}

fn generate_deserialize_impl(ident: &Ident, methods: &[Procedure]) -> proc_macro2::TokenStream {
    let call_inner = prefix(ident, "Call_Inner");
    let call = prefix(ident, "Call");
    let response_variant = prefix(ident, "Call_Response_Variant");
    let response = prefix(ident, "Response");
    let mut arms = proc_macro2::TokenStream::new();
    for (index, method) in methods.iter().enumerate() {
        let ident = &method.ident;
        let mut sig = proc_macro2::TokenStream::new();
        let mut args = proc_macro2::TokenStream::new();
        for index in (0..=method.arg_types.len()).map(|i| i + 1) {
            args.extend(quote! {
                seq.next_element()?.ok_or_else(|| ::serde::de::Error::invalid_length(#index, &self))?,
            });
        }
        sig.extend(quote! {
            (#args)
        });
        arms.extend(quote! {
            #index => {
                #call_inner::#ident#sig
            }
        });
    }
    quote! {
        Ok(#call{
            call: match index {
                #arms,
                _ => {
                    let d: #response = seq.next_element()?.ok_or_else(|| ::serde::de::Error::invalid_length(1, &self))?;
                    #call_inner::#response_variant(d)
                }
            }
        })
    }
}

fn generate_deserialize_return_impl(
    ident: &Ident,
    methods: &[Procedure],
) -> proc_macro2::TokenStream {
    let response = prefix(ident, "Response");
    let mut arms = proc_macro2::TokenStream::new();
    for (index, method) in methods.iter().enumerate() {
        let ident = &method.ident;
        let index = index as u64;
        arms.extend(quote! {
            #index => {
                Ok(#response::#ident(seq.next_element()?.ok_or_else(|| ::serde::de::Error::invalid_length(0, &self))?, seq.next_element()?.ok_or_else(|| ::serde::de::Error::invalid_length(0, &self))?, index))
            }
        });
    }
    quote! {
        match index {
            #arms
            _ => Err(::serde::de::Error::invalid_length(0, &self))?
        }
    }
}

fn generate_shim_forward(methods: &[Procedure]) -> proc_macro2::TokenStream {
    let mut calls = proc_macro2::TokenStream::new();
    for method in methods {
        let ident = &method.ident;
        let mut args = proc_macro2::TokenStream::new();
        let mut arg_names = proc_macro2::TokenStream::new();
        if !method.arg_types.is_empty() {
            for (index, ty) in method.arg_types.iter().enumerate() {
                let ident = Ident::new(&format!("_{}", index), Span::call_site());
                args.extend(quote! {
                    #ident: #ty,
                });
                arg_names.extend(quote! {
                    #ident,
                });
            }
        }
        let receiver = if method.mut_receiver {
            quote! {
                &mut self
            }
        } else {
            quote! {
                &self
            }
        };
        let return_type = &method.return_type;
        calls.extend(quote! {
            fn #ident(#receiver, #args) -> #return_type {
                let ctx = ::vessels::protocol::Context::<<#return_type as ::vessels::protocol::Value>::Item>::new();
                self.inner.#ident(#arg_names)
            }
        });
    }
    calls
}

fn generate_st_traits(ident: &Ident, methods: &[Procedure]) -> proc_macro2::TokenStream {
    let channel = prefix(ident, "Channel");
    let mut items = proc_macro2::TokenStream::new();
    let mut variants = proc_macro2::TokenStream::new();

    methods.iter().for_each(|m| {
        let r_type = m.return_type.as_ref().unwrap();
        let ident = prefix(ident, &format!("METHOD_TRAIT_{}", m.ident.as_ref().unwrap().to_string()));
        items.extend(quote! {
            #[allow(non_camel_case_types)]
            #[doc(hidden)]
            pub trait #ident: ::futures::Stream<Item = <#r_type as ::vessels::protocol::Value>::Item, Error = ()> + ::futures::Sink<SinkItem = <#r_type as ::vessels::protocol::Value>::Item, SinkError = ()> + Send + Sync {}
            impl<T> #ident for T where T: ::futures::Stream<Item = <#r_type as ::vessels::protocol::Value>::Item, Error = ()> + ::futures::Sink<SinkItem = <#r_type as ::vessels::protocol::Value>::Item, SinkError = ()> + Send + Sync {}
        });
        let o_ident = m.ident.as_ref().unwrap();
        variants.extend(quote! {
            #o_ident(Box<dyn #ident>),
        })
    });

    quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub enum #channel {
            #variants
        }
        #items
    }
}

fn generate_handle_response(ident: &Ident, methods: &[Procedure]) -> proc_macro2::TokenStream {
    let channel = prefix(ident, "Channel");
    let response = prefix(ident, "Response");
    let mut arms = proc_macro2::TokenStream::new();
    for method in methods {
        let ident = method.ident.as_ref().unwrap();
        arms.extend(quote! {
            #response::#ident(data, index, id) => {
                let mut channels = self.channels.write().unwrap();
                if let Some(#channel::#ident(channel)) = channels.get_mut(&id) {
                    channel.start_send(data).unwrap();
                }
            }
        });
    }
    quote! {
        match item {
            #arms
        }
    }
}

fn prefix<'a>(ident: &Ident, name: &'a str) -> Ident {
    Ident::new(
        &format!("_{}_PROTOCOL_IMPLEMENTATION_{}", ident, name),
        Span::call_site(),
    )
}

fn generate_binds(ident: &Ident, methods: &[Procedure]) -> TokenStream {
    let enum_variants = generate_enum(methods);
    let return_variants = generate_return_variants(methods);
    let remote_impl = generate_remote_impl(ident, methods);
    let serialize_impl = generate_serialize_impl(ident, methods);
    let serialize_return_impl = generate_serialize_return_impl(ident, methods);
    let deserialize_impl = generate_deserialize_impl(ident, methods);
    let deserialize_return_impl = generate_deserialize_return_impl(ident, methods);
    let blanket = generate_blanket(ident, methods);
    let st_traits = generate_st_traits(ident, methods);
    let handle_response = generate_handle_response(ident, methods);
    let shim_forward = generate_shim_forward(methods);
    let call_repr: proc_macro2::TokenStream;
    let m_len = methods.len();
    let c_remote = prefix(ident, "Concrete_Remote");
    let never_ready = prefix(ident, "Never_Ready");
    let call_inner = prefix(ident, "Call_Inner");
    let protocol_shim = prefix(ident, "Protocol_Shim");
    let protocol_trait = prefix(ident, "Protocol_Trait");
    let call = prefix(ident, "Call");
    let remote = prefix(ident, "Remote");
    let response = prefix(ident, "Response");
    let response_variant = prefix(ident, "Call_Response_Variant");
    let channel = prefix(ident, "Channel");
    if methods.len() == 1 && methods[0].arg_types.is_empty() {
        call_repr = proc_macro2::TokenStream::new();
    } else {
        call_repr = quote! {
            #[repr(transparent)]
        };
    }
    let gen = quote! {
        #[allow(non_snake_case)]
        #[allow(non_camel_case_types)]
        #[derive(Clone)]
        #[allow(non_camel_case_types)]
        struct #c_remote {
            task: ::std::sync::Arc<::futures::task::AtomicTask>,
            queue: ::std::sync::Arc<::std::sync::RwLock<::std::collections::VecDeque<#call>>>,
            ids: ::std::sync::Arc<::std::sync::RwLock<Vec<u64>>>,
            last_id: ::std::sync::Arc<::std::sync::atomic::AtomicU64>,
            channels: ::std::sync::Arc<::std::sync::RwLock<::std::collections::HashMap<u64, #channel>>>,
        }
        impl #c_remote {
            pub fn new() -> #c_remote {
                #c_remote {
                    task: ::std::sync::Arc::new(::futures::task::AtomicTask::new()),
                    queue: ::std::sync::Arc::new(::std::sync::RwLock::new(::std::collections::VecDeque::new())),
                    ids: ::std::sync::Arc::new(::std::sync::RwLock::new(vec![])),
                    last_id: ::std::sync::Arc::new(::std::sync::atomic::AtomicU64::new(0)),
                    channels: ::std::sync::Arc::new(::std::sync::RwLock::new(::std::collections::HashMap::new())),
                }
            }
            fn next_id(&self) -> u64 {
                let mut ids = self.ids.write().unwrap();
                if let Some(id) = ids.pop() {
                    id
                } else {
                    self.last_id.fetch_add(1, ::std::sync::atomic::Ordering::SeqCst)
                }
            }
        }
        impl #ident for #c_remote {
            #remote_impl
        }
        impl ::futures::Stream for #c_remote {
            type Item = #call;
            type Error = ();

            fn poll(&mut self) -> ::futures::Poll<::std::option::Option<Self::Item>, Self::Error> {
                match self.queue.write().unwrap().pop_front() {
                    Some(item) => {
                        Ok(::futures::Async::Ready(Some(item)))
                    },
                    None => {
                        self.task.register();
                        Ok(::futures::Async::NotReady)
                    }
                }
            }
        }
        impl ::futures::Sink for #c_remote {
            type SinkItem = #response;
            type SinkError = ();

            fn start_send(&mut self, item: Self::SinkItem) -> ::futures::StartSend<Self::SinkItem, Self::SinkError> {
                #handle_response
                Ok(::futures::AsyncSink::Ready)
            }
            fn poll_complete(&mut self) -> ::futures::Poll<(), Self::SinkError> {
                Ok(::futures::Async::Ready(()))
            }
        }
        struct #never_ready<T, E> {
            item: ::std::marker::PhantomData<T>,
            error: ::std::marker::PhantomData<E>
        }
        impl<T, E> #never_ready<T, E> {
            fn new() -> Self {
                #never_ready {
                    item: ::std::marker::PhantomData,
                    error: ::std::marker::PhantomData,
                }
            }
        }
        impl<T, E> ::futures::Stream for #never_ready<T, E> {
            type Item = T;
            type Error = E;

            fn poll(&mut self) -> ::futures::Poll<Option<Self::Item>, Self::Error> {
                Ok(::futures::Async::NotReady)
            }
        }
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        #call_repr
        pub struct #call {
            call: #call_inner,
        }
        #[allow(non_camel_case_types)]
        enum #call_inner {
            #(#enum_variants),*,
            #response_variant(#response)
        }
        #st_traits
        #[allow(non_camel_case_types)]
        #[doc(hidden)]
        pub enum #response {
            #(#return_variants),*
        }
        impl ::serde::Serialize for #call {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: ::serde::Serializer {
                use ::serde::ser::SerializeSeq;
                match &self.call {
                    #serialize_impl
                    #call_inner::#response_variant(response) => {
                        let mut seq = serializer.serialize_seq(Some(4))?;
                        seq.serialize_element(&#m_len)?;
                        seq.serialize_element(response)?;
                        seq.end()
                    }
                }
            }
        }
        impl ::serde::Serialize for #response {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: ::serde::Serializer {
                use ::serde::ser::SerializeSeq;
                match self {
                    #serialize_return_impl
                }
            }
        }
        impl<'de> ::serde::Deserialize<'de> for #call {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: ::serde::Deserializer<'de> {
                struct CallVisitor;
                impl<'de> ::serde::de::Visitor<'de> for CallVisitor {
                    type Value = #call;

                    fn expecting(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                        formatter.write_str("a serialized protocol #call")
                    }
                    fn visit_seq<V>(self, mut seq: V) -> Result<#call, V::Error> where V: ::serde::de::SeqAccess<'de>, {
                        let index: usize = seq.next_element()?.ok_or_else(|| ::serde::de::Error::invalid_length(0, &self))?;
                        #deserialize_impl
                    }
                }
                deserializer.deserialize_seq(CallVisitor)
            }
        }
        trait #remote: futures::Stream<Item = #call, Error = ()> + futures::Sink<SinkItem = #response, SinkError = ()> + Clone {}
        impl #remote for #c_remote {}
        impl<'de> ::serde::Deserialize<'de> for #response {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: ::serde::Deserializer<'de> {
                struct ResponseVisitor;
                impl<'de> ::serde::de::Visitor<'de> for ResponseVisitor {
                    type Value = #response;

                    fn expecting(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                        formatter.write_str("a serialized protocol #response")
                    }
                    fn visit_seq<V>(self, mut seq: V) -> Result<#response, V::Error> where V: ::serde::de::SeqAccess<'de>, {
                        let index: u64 = seq.next_element()?.ok_or_else(|| ::serde::de::Error::invalid_length(0, &self))?;
                        #deserialize_return_impl
                    }
                }
                deserializer.deserialize_seq(ResponseVisitor)
            }
        }
        #[allow(non_camel_case_types)]
        struct #protocol_shim<T: #ident> {
            inner: T,
            channels: ::std::collections::HashMap<u64, #channel>,
            inner_stream: Box<dyn ::futures::Stream<Item = #response, Error = ()> + Send>,
            task: ::std::sync::Arc<::futures::task::AtomicTask>
        }
        impl<T: #ident> #protocol_shim<T> {
            pub fn new(inner: T) -> Self {
                #protocol_shim {
                    inner,
                    channels: ::std::collections::HashMap::new(),
                    inner_stream: Box::new(#never_ready::new()),
                    task: ::std::sync::Arc::new(::futures::task::AtomicTask::new())
                }
            }
        }
        impl<T> ::futures::Sink for #protocol_shim<T> where T: #ident {
            type SinkItem = #call;
            type SinkError = ();
            fn start_send(&mut self, item: Self::SinkItem) -> ::futures::StartSend<Self::SinkItem, Self::SinkError> {
                use ::vessels::protocol::Value;
                use ::futures::{Stream, Sink, Future};
                match item.call {
                    #blanket
                    #call_inner::#response_variant(resp) => {
                        // TODO
                    }
                }
                Ok(::futures::AsyncSink::Ready)
            }
            fn poll_complete(&mut self) -> ::futures::Poll<(), Self::SinkError> {
                Ok(::futures::Async::Ready(()))
            }
        }
        impl<T> ::futures::Stream for #protocol_shim<T> where T: #ident {
            type Item = #response;
            type Error = ();

            fn poll(&mut self) -> ::futures::Poll<Option<Self::Item>, Self::Error> {
                let poll = self.inner_stream.poll();
                if let Ok(::futures::Async::NotReady) = poll {
                    self.task.register();
                }
                poll
            }
        }
        pub trait #protocol_trait: ::futures::Sink<SinkItem = #call, SinkError = ()> + ::futures::Stream<Item = #response, Error = ()> + #ident + Send {}
        #[allow(non_camel_case_types)]
        impl<T> #protocol_trait for #protocol_shim<T> where T: #ident + Send {}
        impl<T: #ident> #ident for #protocol_shim<T> {
            #shim_forward
        }
    };
    gen.into()
}

fn generate_blanket(ident: &Ident, methods: &[Procedure]) -> proc_macro2::TokenStream {
    let call_inner = prefix(ident, "Call_Inner");
    let response = prefix(ident, "Response");
    let mut arms = proc_macro2::TokenStream::new();
    for (index, method) in methods.iter().enumerate() {
        let index = index as u64;
        let ident = &method.ident;
        let mut sig = proc_macro2::TokenStream::new();
        let mut args = proc_macro2::TokenStream::new();
        for index in 0..method.arg_types.len() {
            let ident = Ident::new(&format!("_{}", index), Span::call_site());
            args.extend(quote! {
                #ident,
            });
        }
        let mut s_args = args.clone();
        let id = Ident::new(&format!("_{}", method.arg_types.len()), Span::call_site());
        s_args.extend(quote! {
            #id,
        });
        sig.extend(quote! {
            (#s_args)
        });
        arms.extend(quote! {
            #call_inner::#ident#sig => {
                let (context, loc_context) = ::vessels::protocol::Context::new();
                self.#ident(#args).deconstruct(context);
                let (sink, stream) = loc_context.split();
                let mut i_stream: Box<dyn ::futures::Stream<Error = (), Item = #response> + Send + 'static> = Box::new(futures::stream::empty());
                std::mem::swap(&mut self.inner_stream, &mut i_stream);
                self.inner_stream = Box::new(stream.map(move |i| #response::#ident(i, #index, #id)).select(i_stream));
                self.task.notify();
            }
        });
    }
    arms
}

#[proc_macro_attribute]
pub fn protocol(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return r#"compile_error!("unexpected arguments passed to `protocol`");"#
            .parse()
            .unwrap();
    }
    let mut input = {
        let item = item.clone();
        parse_macro_input!(item as ItemTrait)
    };
    if !input.generics.params.is_empty() {
        return TokenStream::from(quote_spanned! {
            input.generics.params.first().unwrap().span() =>
            compile_error!("generic parameters not allowed in `protocol` trait");
        });
    }
    if !input.supertraits.is_empty() {
        return TokenStream::from(quote_spanned! {
            input.supertraits.first().unwrap().span() =>
            compile_error!("supertraits not allowed on `protocol` trait");
        });
    }
    let mut assert_stream = TokenStream::new();
    let mut procedures = vec![];
    for (index, item) in input.items.iter_mut().enumerate() {
        let mut procedure = Procedure {
            arg_types: vec![],
            return_type: None,
            ident: None,
            mut_receiver: false,
        };
        if let TraitItem::Method(method) = item {
            if &format!("{}", method.sig.ident) == "remote" {
                return TokenStream::from(quote_spanned! {
                    method.sig.ident.span() =>
                    compile_error!("`protocol` methods must not be named remote");
                });
            }
            if &format!("{}", method.sig.ident) == "into_protocol" {
                return TokenStream::from(quote_spanned! {
                    method.sig.ident.span() =>
                    compile_error!("`protocol` methods must not be named into_protocol");
                });
            }
            if let Some(default) = &method.default {
                return TokenStream::from(quote_spanned! {
                    default.span() =>
                    compile_error!("default implementations not allowed in `protocol` methods");
                });
            }
            if !method.sig.decl.generics.params.is_empty() {
                return TokenStream::from(quote_spanned! {
                    method.sig.decl.generics.params.first().unwrap().span() =>
                    compile_error!("generic parameters not allowed on `protocol` method");
                });
            }
            if let Some(where_clause) = &method.sig.decl.generics.where_clause {
                return TokenStream::from(quote_spanned! {
                    where_clause.span() =>
                    compile_error!("where clause not allowed on `protocol` method");
                });
            }
            if let ReturnType::Type(_, ty) = &mut method.sig.decl.output {
                let ident = Ident::new(
                    &format!("_{}_{}_rt_AssertValue", &input.ident, index),
                    Span::call_site(),
                );
                assert_stream.extend(TokenStream::from(quote_spanned! {
                    ty.span() =>
                    #[allow(non_camel_case_types)]
                    struct #ident where #ty: ::vessels::protocol::Value;
                }));
                procedure.return_type = Some(*ty.clone());
            } else {
                let m: proc_macro::TokenStream = quote! {
                    ()
                }
                .into();
                let ty = parse_macro_input!(m as Type);
                procedure.return_type = Some(ty);
            }
            let mut has_receiver = false;
            for (arg_index, argument) in method.sig.decl.inputs.iter().enumerate() {
                match argument {
                    FnArg::SelfValue(_) => {
                        return TokenStream::from(quote_spanned! {
                            argument.span() =>
                            compile_error!("cannot consume self in `protocol` method");
                        });
                    }
                    FnArg::SelfRef(self_ref) => {
                        if self_ref.mutability.is_some() {
                            procedure.mut_receiver = true;
                        }
                        has_receiver = true;
                    }
                    FnArg::Captured(argument) => {
                        let ty = &argument.ty;
                        let ident = Ident::new(
                            &format!(
                                "_{}_{}_arg_{}_AssertSerializeDeserialize",
                                &input.ident, index, arg_index
                            ),
                            Span::call_site(),
                        );
                        assert_stream.extend(TokenStream::from(quote_spanned! {
                            ty.span() =>
                            #[allow(non_camel_case_types)]
                            struct #ident where #ty: ::serde::Serialize + ::serde::de::DeserializeOwned;
                        }));
                        procedure.arg_types.push(argument.ty.clone());
                    }
                    _ => {
                        return TokenStream::from(quote_spanned! {
                            argument.span() =>
                            compile_error!("inferred or ignored argument not allowed in `protocol` method");
                        });
                    }
                };
            }
            if !has_receiver {
                return TokenStream::from(quote_spanned! {
                    method.sig.ident.span() =>
                    compile_error!("method in `protocol` has no receiver");
                });
            }
            procedure.ident = Some(method.sig.ident.clone());
        } else {
            return TokenStream::from(quote_spanned! {
                item.span() =>
                compile_error!("`protocol` expected method");
            });
        }
        procedures.push(procedure);
    }
    if procedures.is_empty() {
        return TokenStream::from(quote_spanned! {
            input.span() =>
            compile_error!("`protocol` with no methods is invalid");
        });
    }
    let ident = &input.ident;
    let protocol_shim = prefix(ident, "Protocol_Shim");
    let protocol_trait = prefix(ident, "Protocol_Trait");
    let mut m: TokenStream = quote! {
        #[doc(hidden)]
        fn into_protocol(self) -> Box<dyn #protocol_trait> where Self: Sized + 'static {
            Box::new(#protocol_shim::new(self))
        }
    }
    .into();
    input
        .items
        .push(TraitItem::Method(parse_macro_input!(m as TraitItemMethod)));
    m = quote! {
        #[doc(hidden)]
        fn IS_PROTO() where Self: Sized {}
    }
    .into();
    input
        .items
        .push(TraitItem::Method(parse_macro_input!(m as TraitItemMethod)));
    let mut ty_path = Punctuated::new();
    ty_path.push_value(PathSegment {
        arguments: PathArguments::None,
        ident: Ident::new("Send", input.ident.span()),
    });
    input
        .supertraits
        .push_value(TypeParamBound::Trait(TraitBound {
            paren_token: None,
            modifier: TraitBoundModifier::None,
            lifetimes: None,
            path: Path {
                leading_colon: None,
                segments: ty_path,
            },
        }));
    let c_remote = prefix(ident, "Concrete_Remote");
    let remote = prefix(ident, "Remote");
    let binds = generate_binds(ident, &procedures);
    let blanket_impl: TokenStream = quote! {
        impl dyn #ident {
            fn remote() -> impl #ident + #remote {
                #c_remote::new()
            }
        }
    }
    .into();
    let mut item: TokenStream = input.into_token_stream().into();
    item.extend(blanket_impl);
    item.extend(assert_stream);
    item.extend(binds);
    item
}

decl_derive!([Value] => value_derive);

fn value_derive(mut s: synstructure::Structure) -> proc_macro2::TokenStream {
    let ast = s.ast();
    let ident = &ast.ident;
    if s.variants().is_empty() {
        return quote_spanned! {
            ident.span() =>
            compile_error!("Value cannot be derived for an enum with no variants");
        };
    }
    let en = prefix(ident, "Derive_Variants");
    let mut stream = proc_macro2::TokenStream::new();
    let mut variants = proc_macro2::TokenStream::new();
    let mut serialize_impl = proc_macro2::TokenStream::new();
    let mut deserialize_impl = proc_macro2::TokenStream::new();
    let mut id: usize = 0;
    s.variants().iter().for_each(|variant| {
        let ident = &variant.ast().ident;
        let base = format!("{}_AssertValue_", ident);
        let bindings = variant.bindings();
        bindings.iter().enumerate().for_each(|(index, binding)| {
            let name = prefix(&ast.ident, &(base.clone() + &index.to_string()));
            let ident = Ident::new(&format!("{}_{}", ident, index), Span::call_site());
            let ty = &binding.ast().ty;
            variants.extend(quote! {
                #ident(<#ty as ::vessels::protocol::Value>::Item),
            });
            stream.extend(quote! {
                struct #name where #ty: ::vessels::protocol::Value;
            });
            serialize_impl.extend(quote! {
                #en::#ident(data) => {
                    let mut seq = serializer.serialize_seq(Some(2))?;
                    seq.serialize_element(&#id)?;
                    seq.serialize_element(data)?;
                    seq.end()
                }
            });
            deserialize_impl.extend(quote! {
                #id => {
                    #en::#ident(seq.next_element()?.ok_or_else(|| ::serde::de::Error::invalid_length(1, &self))?)
                }
            });
            id += 1;
        });
        if bindings.is_empty() {
            variants.extend(quote! {
                #ident,
            });
            serialize_impl.extend(quote! {
                #en::#ident => {
                    let mut seq = serializer.serialize_seq(Some(1))?;
                    seq.serialize_element(&#id)?;
                    seq.end()
                }
            });
            deserialize_impl.extend(quote! {
                #id => {
                    #en::#ident
                }
            });
        }
    });
    let expectation = format!("a serialized Value item from the derivation on {}", ident);
    stream.extend(quote! {
        #[doc(hidden)]
        pub enum #en {
            #variants
        }
        impl ::serde::Serialize for #en {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: ::serde::Serializer {
                use ::serde::ser::SerializeSeq;
                match self {
                    #serialize_impl
                }
            }
        }
        impl<'de> ::serde::Deserialize<'de> for #en {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: ::serde::Deserializer<'de> {
                struct CallVisitor;
                impl<'de> ::serde::de::Visitor<'de> for CallVisitor {
                    type Value = #en;

                    fn expecting(&self, formatter: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                        formatter.write_str(#expectation)
                    }
                    fn visit_seq<V>(self, mut seq: V) -> Result<#en, V::Error> where V: ::serde::de::SeqAccess<'de>, {
                        let index: usize = seq.next_element()?.ok_or_else(|| ::serde::de::Error::invalid_length(0, &self))?;
                        Ok(match index {
                            #deserialize_impl
                            _ => { Err(::serde::de::Error::invalid_length(0, &self))? }
                        })
                    }
                }
                deserializer.deserialize_seq(CallVisitor)
            }
        }
    });
    s.bind_with(|_| synstructure::BindStyle::Move);
    let mut return_stream = proc_macro2::TokenStream::new();
    let mut decl_stream = proc_macro2::TokenStream::new();
    let mut select_stream = proc_macro2::TokenStream::new();
    let mut idx = 0;
    let deconstruct = s.each_variant(|variant| {
        let ident = &variant.ast().ident;
        let bindings = variant.bindings();
        if bindings.is_empty() {
            return quote! {
                sink.start_send(#en::#ident).unwrap();
            };
        };
        let mut stream = proc_macro2::TokenStream::new();
        bindings.iter().enumerate().for_each(|(index, bi)| {
            let pat = &bi.pat();
            let ty = &bi.ast().ty;
            let r_ident = Ident::new(&format!("{}_{}", ident, index), Span::call_site());
            let ident = Ident::new(&format!("{}_{}_ct", ident, index), Span::call_site());
            let ident_ctx = Ident::new(&format!("{}_{}_ctx", ident, index), Span::call_site());
            stream.extend(quote! {
                let ctxs = ::vessels::protocol::Context::new();
                let (i_sink, i_stream) = ctxs.1.split();
                #ident_ctx = Some(i_sink);
                #ident = Some(i_stream);
                #pat.deconstruct(ctxs.0);
            });
            return_stream.extend(quote! {
                #en::#r_ident(data) => {
                    let mut s = None;
                    ::std::mem::swap(&mut s, &mut #ident_ctx);
                    ::vessels::executor::spawn(s.expect("No split sink").send_all(::futures::stream::once(Ok(data)).chain(item.1.filter_map(|item| {
                        if let #en::#r_ident(item) = item {
                            Some(item)
                        } else {
                            None
                        }
                    }))).map_err(|e| {
                        println!("{:?}", e);
                        e
                    }).then(|_| Ok(())));
                }
            });
            select_stream.extend(quote! {
                let sel_stream = (if let Some(stream) = #ident { Box::new(stream.map(|item| #en::#r_ident(item)).select(sel_stream)) } else { sel_stream });
            });
            decl_stream.extend(quote! {
                let (mut #ident, mut #ident_ctx): (Option<::futures::stream::SplitStream<::vessels::protocol::Context::<<#ty as ::vessels::protocol::Value>::Item>>>, Option<::futures::stream::SplitSink<::vessels::protocol::Context::<<#ty as ::vessels::protocol::Value>::Item>>>) = (None, None);
            });
            idx += 1;
        });
        stream
    });
    let mut construct = proc_macro2::TokenStream::new();
    s.variants().iter().for_each(|variant| {
        let v_ident = variant.ast().ident;
        let pat = &variant.pat();
        let bindings = variant.bindings();
        if bindings.is_empty() {
            construct.extend(quote! {
                #en::#v_ident => { Ok(#pat) }
            });
            return;
        }
        (0..bindings.len()).for_each(|index| {
            let mut decl_stream = proc_macro2::TokenStream::new();
            let mut select_stream = proc_macro2::TokenStream::new();
            let mut item_stream = proc_macro2::TokenStream::new();
            let b_ident = Ident::new(&format!("{}_{}", v_ident, index), Span::call_site());
            let cst = variant.construct(|field, idx| {
                let b_i_ident = Ident::new(&format!("{}_{}", v_ident, idx), Span::call_site());
                let ident_ct = Ident::new(&format!("{}_{}_ct", v_ident, idx), Span::call_site());
                let ident_ctx = Ident::new(&format!("{}_{}_ctx", ident_ct, idx), Span::call_site());
                let ident_ctxs = Ident::new(&format!("{}_{}_ctxs", ident_ct, idx), Span::call_site());
                let ty = &field.ty;
                decl_stream.extend(quote! {
                    let (mut #ident_ct, mut #ident_ctx): (::futures::stream::SplitStream<::vessels::protocol::Context::<<#ty as ::vessels::protocol::Value>::Item>>, ::futures::stream::SplitSink<::vessels::protocol::Context::<<#ty as ::vessels::protocol::Value>::Item>>);
                });
                select_stream.extend(quote! {
                    let sel_stream = #ident_ct.map(|item| #en::#b_i_ident(item)).select(sel_stream);
                });
                item_stream.extend(quote! {
                    #en::#b_i_ident(item) => {
                        #ident_ctx.start_send(item).unwrap();
                    }
                });
                quote! {
                    {
                        let ret = <#ty as ::vessels::protocol::Value>::construct(#ident_ctxs);
                        ret
                    }
                }
            });
            let mut mcst = proc_macro2::TokenStream::new();
            variant.bindings().iter().enumerate().for_each(|(idx, field)| {
                let ident_ct = Ident::new(&format!("{}_{}_ct", v_ident, idx), Span::call_site());
                let ident_ctx = Ident::new(&format!("{}_{}_ctx", ident_ct, idx), Span::call_site());
                let ident_ctxs = Ident::new(&format!("{}_{}_ctxs", ident_ct, idx), Span::call_site());
                let ty = &field.ast().ty;
                decl_stream.extend(quote! {
                    let #ident_ctxs: ::vessels::protocol::Context<<#ty as ::vessels::protocol::Value>::Item>;
                });
                mcst.extend(quote! {
                    {
                        let ctxs = ::vessels::protocol::Context::new();
                        let (i_sink, i_stream) = ctxs.1.split();
                        #ident_ctx = i_sink;
                        #ident_ct = i_stream;
                        #ident_ctxs = ctxs.0;
                    }
                });
            });
            construct.extend(quote! {
                #en::#b_ident(data) => {
                    let sel_stream = ::futures::stream::empty();
                    #decl_stream
                    #mcst;
                    ::vessels::executor::spawn(::futures::stream::once(Ok(#en::#b_ident(data))).chain(v.1).for_each(move |item| {
                        match item {
                            #item_stream
                            _ => {}
                        };
                        Ok(())
                    }));
                    #select_stream
                    ::vessels::executor::spawn(sel_stream.forward(sink).map_err(|e| {
                        println!("{:?}", e);
                        e
                    }).then(|_| Ok(())));
                    let ret = Ok(#cst);
                    ret
                }
            });
        });
    });
    let wrapper_ident = prefix(ident, "Derive_Container");
    stream.extend(quote! {
        impl ::vessels::protocol::Value for #ident {
            type Item = #en;

            fn deconstruct<
                C: ::futures::Sink<SinkItem = Self::Item, SinkError = ()>
                    + ::futures::Stream<Item = Self::Item, Error = ()>
                    + Send
                    + 'static,
            >(
                self,
                context: C,
            ) where
                Self: Sized,
            {
                use ::futures::{Sink, Stream};
                let (mut sink, mut stream) = context.split();
                let sel_stream: Box<dyn Stream<Item = Self::Item, Error = ()> + Send> = Box::new(::futures::stream::empty());
                #decl_stream
                match self {
                    #deconstruct
                };
                ::vessels::executor::spawn(stream.into_future().map_err(|e| {
                        println!("{:?}", e.0);
                        ()
                    }).and_then(move |item| {
                        let i = item.0.unwrap();
                        match i {
                            #return_stream
                            _ => {}
                        };
                        Ok(())
                    }
                ));
                #select_stream
                ::vessels::executor::spawn(sel_stream.forward(sink).map_err(|e| {
                        println!("{:?}", e);
                        e
                    }).then(|_| Ok(())));
            }
            fn construct<
                C: ::futures::Sink<SinkItem = Self::Item, SinkError = ()>
                    + ::futures::Stream<Item = Self::Item, Error = ()>
                    + Send
                    + 'static,
            >(
                context: C,
            ) -> Self {
                use ::futures::{Sink, Stream};
                let (sink, stream) = context.split();
                if let Ok(constructed) = stream.into_future().and_then(|v| {
                    match v.0.unwrap() {
                        #construct
                    }
                }).wait() {
                    constructed
                } else {
                    panic!("Invalid return in derived Value construction")
                }
                
            }
        }
    });
    quote! {
        const #wrapper_ident: () = {
            #stream
        };
    }
}
