use crate::Kind;
use failure::Fail;
use std::{
    any::{Any, TypeId},
    fmt::{self, Display, Formatter},
};

pub type MethodIndex = u8;

pub struct MethodTypes {
    pub arguments: Vec<TypeId>,
    pub output: TypeId,
    pub receiver: Receiver,
}

#[derive(Debug, Fail)]
pub enum CallError {
    Type(u8),
    ArgumentCount(#[fail(cause)] ArgumentCountError),
    OutOfRange(#[fail(cause)] OutOfRangeError),
    IncorrectReceiver(Receiver),
}

impl Display for CallError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        use CallError::{ArgumentCount, IncorrectReceiver, OutOfRange, Type};

        write!(
            f,
            "{}",
            match self {
                Type(position) => format!("invalid type for argument {}", position),
                OutOfRange(error) => format!("{}", error),
                ArgumentCount(error) => format!("{}", error),
                IncorrectReceiver(expected) => format!("expected {} receiver", expected),
            }
        )
    }
}

#[derive(Debug, Fail)]
#[fail(display = "method {} out of range", index)]
pub struct OutOfRangeError {
    pub index: MethodIndex,
}

#[derive(Debug, Fail)]
#[fail(display = "got {} arguments, expected {}", got, expected)]
pub struct ArgumentCountError {
    pub expected: usize,
    pub got: usize,
}

#[derive(Debug, Fail)]
#[fail(display = "no method with name {}", name)]
pub struct NameError {
    pub name: String,
}

#[derive(Debug, Fail)]
#[fail(display = "cannot cast to {:?} in this context", target)]
pub struct CastError {
    pub target: TypeId,
}

/// A trait object that has reflection data generated by `#[object]`
///
/// This trait should not be implemented manually by any third-party crate. Trait objects of traits annotated
/// with `#[object]` will be marked with this trait to indicate their usability as reflected trait objects.
/// Moreover, any type that implements `Trait<dyn SomeTrait>` where `dyn SomeTrait: Reflected` will
/// have a generated implementation allowing it to satisfy `SomeTrait`.
pub trait Reflected: 'static {
    #[doc(hidden)]
    type Shim: Kind;
    #[doc(hidden)]
    type ErasedShim: From<Box<Self>>;
    #[doc(hidden)]
    const DO_NOT_IMPLEMENT_THIS_MARKER_TRAIT_MANUALLY: ();
}

#[derive(Debug)]
pub enum Receiver {
    Mutable,
    Immutable,
    Owned,
}

impl Display for Receiver {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        use Receiver::{Immutable, Mutable, Owned};

        write!(
            f,
            "{}",
            match self {
                Immutable => "an immutable",
                Mutable => "a mutable",
                Owned => "an owned",
            }
        )
    }
}

impl Receiver {
    pub fn is_mutable(&self) -> bool {
        use Receiver::Mutable;
        if let Mutable = self {
            true
        } else {
            false
        }
    }
}

pub enum SomeTrait {}

impl Reflected for SomeTrait {
    type Shim = ();
    type ErasedShim = ();
    const DO_NOT_IMPLEMENT_THIS_MARKER_TRAIT_MANUALLY: () = ();
}

impl From<Box<SomeTrait>> for () {
    fn from(_: Box<SomeTrait>) {}
}

pub trait Erased: Send + Trait<SomeTrait> {
    fn cast(self: Box<Self>, ty: TypeId) -> Result<Box<dyn Any + Send>, CastError>;
}

pub trait Cast<T: ?Sized + Reflected> {
    fn downcast(self) -> Result<Box<T>, CastError>;
    fn upcast(self) -> Result<Box<T>, CastError>;
}

impl<S: ?Sized + Reflected> Cast<S> for Box<dyn Erased> {
    fn downcast(self) -> Result<Box<S>, CastError> {
        self.cast(TypeId::of::<S>()).map(|erased| {
            *Box::<dyn Any + Send>::downcast::<Box<S>>(erased)
                .map_err(|_| panic!("could not downcast after successful reinterpretation"))
                .unwrap()
        })
    }
    fn upcast(self) -> Result<Box<S>, CastError> {
        Trait::<SomeTrait>::upcast_erased(self, TypeId::of::<S>()).map(|erased| {
            erased
                .downcast()
                .expect("could not downcast after successful upcast")
        })
    }
}

impl<T: ?Sized + Reflected + Trait<T>, S: ?Sized + Reflected> Cast<S> for Box<T> {
    fn downcast(self) -> Result<Box<S>, CastError> {
        self.erase().cast(TypeId::of::<S>()).map(|erased| {
            *Box::<dyn Any + Send>::downcast::<Box<S>>(erased)
                .map_err(|_| panic!("could not downcast after successful reinterpretation"))
                .unwrap()
        })
    }
    fn upcast(self) -> Result<Box<S>, CastError> {
        Trait::<T>::upcast_erased(self, TypeId::of::<S>()).map(|erased| {
            erased
                .downcast()
                .expect("could not downcast after successful upcast")
        })
    }
}

pub trait Trait<T: Reflected + ?Sized> {
    fn call(
        &self,
        index: MethodIndex,
        args: Vec<Box<dyn Any + Send>>,
    ) -> Result<Box<dyn Any + Send>, CallError>;
    fn call_mut(
        &mut self,
        index: MethodIndex,
        args: Vec<Box<dyn Any + Send>>,
    ) -> Result<Box<dyn Any + Send>, CallError>;
    fn call_move(
        self: Box<Self>,
        index: MethodIndex,
        args: Vec<Box<dyn Any + Send>>,
    ) -> Result<Box<dyn Any + Send>, CallError>;
    fn by_name(&self, name: &'_ str) -> Result<MethodIndex, NameError>;
    fn count(&self) -> MethodIndex;
    fn name_of(&self, index: MethodIndex) -> Result<String, OutOfRangeError>;
    fn this(&self) -> TypeId;
    fn name(&self) -> String;
    fn types(&self, index: MethodIndex) -> Result<MethodTypes, OutOfRangeError>;
    /// Returns all supertraits in the form `TypeId::of<dyn SomeTrait>` for each supertrait `SomeTrait`.
    fn supertraits(&self) -> Vec<TypeId>;
    /// For a `TypeId` that is `TypeId::of<dyn SomeTrait>` returns the erasure of a concrete type
    /// `Box<dyn SomeTrait>` which can then be downcasted into.
    fn upcast_erased(self: Box<Self>, ty: TypeId) -> Result<Box<dyn Erased>, CastError>;
    fn erase(self: Box<Self>) -> Box<dyn Erased>;
}
