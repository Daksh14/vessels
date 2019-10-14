use derive::value;
use erased_serde::Serialize as ErasedSerialize;
use failure::Error;
use futures::{
    future::{empty, ok},
    sync::mpsc::{unbounded, UnboundedReceiver, UnboundedSender},
    Future as IFuture, Poll, Sink, StartSend, Stream,
};
use lazy_static::lazy_static;
use serde::{
    de::{DeserializeOwned, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor},
    ser::{SerializeMap, SerializeSeq},
    Deserialize, Serialize, Serializer,
};
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    ffi::{CString, OsString},
    fmt,
    marker::PhantomData,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    num::{
        NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI8, NonZeroIsize, NonZeroU16, NonZeroU32,
        NonZeroU64, NonZeroU8, NonZeroUsize,
    },
    ops::Deref,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, SystemTime},
};

lazy_static! {
    static ref IDX: AtomicU64 = AtomicU64::new(0);
    static ref CHANNELS: Mutex<HashMap<u64, [TypeId; 2]>> = Mutex::new(HashMap::new());
}

pub struct Item {
    ty: TypeId,
    func: DeserializeFn,
}

impl Item {
    fn new(ty: TypeId, func: DeserializeFn) -> Self {
        Item { ty, func }
    }
}

type DeserializeFn =
    fn(&mut dyn erased_serde::Deserializer) -> erased_serde::Result<Box<dyn SerdeAny>>;

inventory::collect!(Item);

#[derive(Serialize, Deserialize)]
pub struct ForkRef(u64);

pub trait Fork: Send + 'static {
    fn fork<V: Value>(&self, value: V) -> ForkRef;
    fn get_fork<V: Value>(
        &self,
        fork_ref: ForkRef,
    ) -> Box<dyn IFuture<Item = V, Error = ()> + Send + 'static>;
}

pub trait Channel<
    I: Serialize + DeserializeOwned + Send + 'static,
    O: Serialize + DeserializeOwned + Send + 'static,
>: Stream<Item = I, Error = ()> + Sink<SinkItem = O, SinkError = ()> + Fork
{
    type ForkFactory: Fork;

    fn split_factory(&self) -> Self::ForkFactory;
}

pub trait Target {
    fn new_with<V: Value>(value: V) -> Self;
    fn value<V: Value>(self) -> V;
}

pub trait Value: Send + 'static {
    type ConstructItem: Serialize + DeserializeOwned + Send + 'static;
    fn construct<C: Channel<Self::ConstructItem, Self::DeconstructItem>>(
        channel: C,
    ) -> Box<dyn IFuture<Item = Self, Error = Error> + Send + 'static>
    where
        Self: Sized;

    type DeconstructItem: Serialize + DeserializeOwned + Send + 'static;
    fn deconstruct<C: Channel<Self::DeconstructItem, Self::ConstructItem>>(
        self,
        channel: C,
    ) -> Box<dyn IFuture<Item = (), Error = ()> + Send + 'static>;

    #[doc(hidden)]
    const DO_NOT_IMPLEMENT_THIS_TRAIT_MANUALLY: ();

    fn on_to<T: Target>(self) -> T
    where
        Self: Sized,
    {
        T::new_with(self)
    }

    fn of<T: Target>(target: T) -> Self
    where
        Self: Sized,
    {
        target.value()
    }
}

#[value]
impl Value for () {
    type ConstructItem = ();
    type DeconstructItem = ();

    fn deconstruct<C: Channel<Self::DeconstructItem, Self::ConstructItem>>(
        self,
        _: C,
    ) -> Box<dyn IFuture<Item = (), Error = ()> + Send + 'static> {
        Box::new(ok(()))
    }
    fn construct<C: Channel<Self::ConstructItem, Self::DeconstructItem>>(
        _: C,
    ) -> Box<dyn IFuture<Item = Self, Error = Error> + Send + 'static> {
        Box::new(ok(()))
    }
}

#[value]
impl<T: Send + 'static> Value for PhantomData<T> {
    type ConstructItem = ();
    type DeconstructItem = ();

    fn deconstruct<C: Channel<Self::DeconstructItem, Self::ConstructItem>>(
        self,
        _: C,
    ) -> Box<dyn IFuture<Item = (), Error = ()> + Send + 'static> {
        Box::new(ok(()))
    }
    fn construct<C: Channel<Self::ConstructItem, Self::DeconstructItem>>(
        _: C,
    ) -> Box<dyn IFuture<Item = Self, Error = Error> + Send + 'static> {
        Box::new(ok(PhantomData))
    }
}

macro_rules! primitive_impl {
    ($($ty:ident)+) => {$(
        #[value]
        impl Value for $ty {
            type ConstructItem = $ty;
            type DeconstructItem = ();

            fn deconstruct<C: Channel<Self::DeconstructItem, Self::ConstructItem>>(
                self,
                channel: C,
            ) -> Box<dyn IFuture<Item = (), Error = ()> + Send + 'static> {
                Box::new(channel.send(self).then(|_| Ok(())))
            }
            fn construct<C: Channel<Self::ConstructItem, Self::DeconstructItem>>(
                channel: C,
            ) -> Box<dyn IFuture<Item = Self, Error = Error> + Send + 'static>
            where
                Self: Sized,
            {
                Box::new(
                    channel
                        .into_future()
                        .map_err(|_| failure::err_msg("test"))
                        .map(|v| v.0.unwrap()),
                )
            }
        }
    )+};
}

primitive_impl!(bool isize i8 i16 i32 i64 usize u8 u16 u32 u64 f32 f64 char CString String Ipv4Addr SocketAddrV4 SocketAddrV6 SocketAddr SystemTime OsString Ipv6Addr Duration NonZeroU8 NonZeroU16 NonZeroU32 NonZeroU64 NonZeroUsize NonZeroI8 NonZeroI16 NonZeroI32 NonZeroI64 NonZeroIsize);

pub struct Serde<T: Serialize + DeserializeOwned + Send + 'static>(T);

impl<T> From<T> for Serde<T>
where
    T: Serialize + DeserializeOwned + Send + 'static,
{
    fn from(input: T) -> Self {
        Serde(input)
    }
}

impl<T: Serialize + DeserializeOwned + Send + 'static> Deref for Serde<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/*#[value]
impl<T> Value for Serde<T>
where
    T: Serialize + DeserializeOwned + Send + 'static,
{
    type ConstructItem = T;
    type DeconstructItem = ();
    fn deconstruct<C: Channel<Self::DeconstructItem, Self::ConstructItem>>(
        self,
        channel: C,
    ) -> Box<dyn IFuture<Item = (), Error = ()> + Send + 'static> {
        Box::new(channel.send(self.0).then(|_| Ok(())))
    }
    fn construct<C: Channel<Self::ConstructItem, Self::DeconstructItem>>(
        channel: C,
    ) -> Box<dyn IFuture<Item = Self, Error = Error> + Send + 'static>
    where
        Self: Sized,
    {
        Box::new(
            channel
                .into_future()
                .map_err(|_| failure::err_msg("test"))
                .map(|v| Serde(v.0.unwrap())),
        )
    }
}*/

pub struct Future<T, E>(Box<dyn IFuture<Item = T, Error = E> + Send + 'static>)
where
    T: Value,
    E: Value;

impl<T: Value, E: Value> Deref for Future<T, E> {
    type Target = Box<dyn IFuture<Item = T, Error = E> + Send + 'static>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<F> From<F> for Future<F::Item, F::Error>
where
    F: IFuture + Send + 'static,
    F::Error: Value,
    F::Item: Value,
{
    fn from(input: F) -> Self {
        Future(Box::new(input))
    }
}

#[derive(Serialize, Deserialize)]
pub enum FResult {
    Ok(ForkRef),
    Err(ForkRef),
}

#[value]
impl<T, E> Value for Future<T, E>
where
    T: Value,
    E: Value,
{
    type ConstructItem = FResult;
    type DeconstructItem = ();
    fn deconstruct<C: Channel<Self::DeconstructItem, Self::ConstructItem>>(
        self,
        channel: C,
    ) -> Box<dyn IFuture<Item = (), Error = ()> + Send + 'static> {
        Box::new(self.0.then(|v| {
            let fork_factory = channel.split_factory();
            channel
                .send(match v {
                    Ok(v) => FResult::Ok(fork_factory.fork(v)),
                    Err(v) => FResult::Err(fork_factory.fork(v)),
                })
                .then(|_| Ok(()))
        }))
    }
    fn construct<C: Channel<Self::ConstructItem, Self::DeconstructItem>>(
        channel: C,
    ) -> Box<dyn IFuture<Item = Self, Error = Error> + Send + 'static>
    where
        Self: Sized,
    {
        Box::new(channel.into_future().then(|v| match v {
            Ok(v) => ok(match v.0.unwrap() {
                FResult::Ok(r) => Future::<T, E>::from(v.1.get_fork::<T>(r).map_err(|_| panic!())),
                FResult::Err(r) => {
                    Future::<T, E>::from(v.1.get_fork::<E>(r).then(|v| Err(v.unwrap())))
                }
            }),
            _ => panic!("lol"),
        }))
    }
}

pub struct IdChannel {
    out_channel: Box<dyn Stream<Item = ChannelItem, Error = ()> + Send>,
}

impl Stream for IdChannel {
    type Item = ChannelItem;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.out_channel.poll()
    }
}

pub trait SerdeAny: erased_serde::Serialize + Any + Send {}

serialize_trait_object!(SerdeAny);

impl<T: ?Sized> SerdeAny for T where T: ErasedSerialize + Any + Send {}

pub struct ChannelItem(pub u64, pub Box<dyn SerdeAny>);

impl Serialize for ChannelItem {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            let mut map = serializer.serialize_map(Some(2))?;
            map.serialize_entry("channel", &self.0)?;
            map.serialize_entry("data", self.1.as_ref())?;
            map.end()
        } else {
            let mut seq = serializer.serialize_seq(Some(2))?;
            seq.serialize_element(&self.0)?;
            seq.serialize_element(self.1.as_ref())?;
            seq.end()
        }
    }
}

struct ItemVisitor;

impl<'de> Visitor<'de> for ItemVisitor {
    type Value = ChannelItem;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "a channel item")
    }

    /*fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
    }*/

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut channel: Option<u64> = None;
        let mut data = None;
        while let Some(key) = map.next_key()? {
            match key {
                "channel" => {
                    if channel.is_some() {
                        return Err(serde::de::Error::duplicate_field("channel"));
                    }
                    channel = Some(map.next_value()?);
                }
                "data" => {
                    if data.is_some() {
                        return Err(serde::de::Error::duplicate_field("data"));
                    }
                    data = Some(map.next_value_seed(Id(channel.unwrap()))?);
                }
                _ => panic!(),
            }
        }
        let channel = channel.ok_or_else(|| serde::de::Error::missing_field("channel"))?;
        let data = data.ok_or_else(|| serde::de::Error::missing_field("data"))?;
        Ok(ChannelItem(channel, data))
    }
}

struct Id(u64);

impl<'de> DeserializeSeed<'de> for Id {
    type Value = Box<dyn SerdeAny>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ty = { *CHANNELS.lock().unwrap().get(&self.0).unwrap() };
        let deserializer = &mut erased_serde::Deserializer::erase(deserializer)
            as &mut dyn erased_serde::Deserializer;
        (inventory::iter::<Item>
            .into_iter()
            .find(|item| item.ty == ty[0])
            .unwrap()
            .func)(deserializer)
        .map_err(|_| panic!())
    }
}

impl<'de> Deserialize<'de> for ChannelItem {
    fn deserialize<D>(deserializer: D) -> Result<ChannelItem, D::Error>
    where
        D: Deserializer<'de>,
    {
        let deserializer = &mut erased_serde::Deserializer::erase(deserializer)
            as &mut dyn erased_serde::Deserializer;
        if deserializer.is_human_readable() {
            deserializer.deserialize_map(ItemVisitor).map_err(|e| {
                println!("{:?}", e);
                panic!();
            })
        } else {
            deserializer.deserialize_seq(ItemVisitor).map_err(|e| {
                println!("{:?}", e);
                panic!();
            })
        }
    }
}

impl Sink for IdChannel {
    type SinkItem = ChannelItem;
    type SinkError = ();

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        Err(())
    }
    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        Err(())
    }
}

impl Target for IdChannel {
    fn new_with<V: Value>(value: V) -> Self {
        let first_channel = IDX.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = IdChannelFork::new_with(value);

        CHANNELS.lock().unwrap().insert(
            first_channel,
            [
                TypeId::of::<V::ConstructItem>(),
                TypeId::of::<V::DeconstructItem>(),
            ],
        );

        IdChannel {
            out_channel: Box::new(
                receiver.map(move |v| ChannelItem(first_channel, Box::new(v) as Box<dyn SerdeAny>)),
            ),
        }
    }

    fn value<V: Value>(self) -> V {
        panic!()
    }
}

impl<
        I: Serialize + DeserializeOwned + Send + 'static,
        O: Serialize + DeserializeOwned + Send + 'static,
    > Fork for IdChannelFork<I, O>
{
    fn fork<V: Value>(&self, value: V) -> ForkRef {
        ForkRef(0)
    }
    fn get_fork<V: Value>(
        &self,
        fork_ref: ForkRef,
    ) -> Box<dyn IFuture<Item = V, Error = ()> + Send + 'static> {
        Box::new(empty())
    }
}

pub struct IdChannelFork<
    I: Serialize + DeserializeOwned + Send + 'static,
    O: Serialize + DeserializeOwned + Send + 'static,
> {
    i: UnboundedReceiver<I>,
    o: UnboundedSender<O>,
}

impl<
        I: Serialize + DeserializeOwned + Send + 'static,
        O: Serialize + DeserializeOwned + Send + 'static,
    > Stream for IdChannelFork<I, O>
{
    type Item = I;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.i.poll().map_err(|_| ())
    }
}

struct SinkStream<T: Stream, U: Sink>(T, U);

impl<T: Stream, U: Sink> Sink for SinkStream<T, U> {
    type SinkItem = U::SinkItem;
    type SinkError = U::SinkError;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        self.1.start_send(item)
    }
    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        self.1.poll_complete()
    }
}

impl<T: Stream, U: Sink> Stream for SinkStream<T, U> {
    type Item = T::Item;
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.0.poll()
    }
}

impl<
        I: Serialize + DeserializeOwned + Send + 'static,
        O: Serialize + DeserializeOwned + Send + 'static,
    > IdChannelFork<I, O>
{
    fn new_with<V: Value<DeconstructItem = I, ConstructItem = O>>(
        value: V,
    ) -> (UnboundedSender<I>, UnboundedReceiver<O>) {
        let (o, oo): (UnboundedSender<I>, UnboundedReceiver<I>) = unbounded();
        let (oi, i): (UnboundedSender<O>, UnboundedReceiver<O>) = unbounded();
        tokio::spawn(value.deconstruct(IdChannelFork { o: oi, i: oo }));
        (o, i)
    }
}

impl<
        I: Serialize + DeserializeOwned + Send + 'static,
        O: Serialize + DeserializeOwned + Send + 'static,
    > Sink for IdChannelFork<I, O>
{
    type SinkItem = O;
    type SinkError = ();

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        self.o.start_send(item).map_err(|_| ())
    }
    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        self.o.poll_complete().map_err(|_| ())
    }
}

impl<
        I: Serialize + DeserializeOwned + Send + 'static,
        O: Serialize + DeserializeOwned + Send + 'static,
    > Channel<I, O> for IdChannelFork<I, O>
{
    type ForkFactory = IdChannelFork<I, O>;

    fn split_factory(&self) -> Self::ForkFactory {
        panic!()
    }
}
