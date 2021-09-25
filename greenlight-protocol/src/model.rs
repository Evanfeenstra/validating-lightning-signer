use core::fmt;
use core::marker::PhantomData;

use serde::{de, ser, Serializer};
use serde_derive::{Deserialize, Serialize};
use serde::ser::SerializeTuple;
use core::fmt::Debug;
use std::fmt::Formatter;

#[derive(Debug, Serialize, Deserialize)]
pub struct Bip32KeyVersion {
    pubkey_version: u32,
    privkey_version : u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BlockID([u8; 32]);

#[derive(Debug, Serialize, Deserialize)]
pub struct Secret([u8; 32]);

#[derive(Debug, Serialize, Deserialize)]
pub struct PrivKey([u8; 32]);

#[derive(Debug, Serialize, Deserialize)]
pub struct PubKey32([u8; 32]);

macro_rules! array_impl {
    ($ty:ident, $len:tt) => {
        pub struct $ty(pub [u8; $len]);

        impl Debug for $ty {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                self.0.to_vec().fmt(f)
            }
        }

        impl<'de> de::Deserialize<'de> for $ty {
            fn deserialize<D>(d: D) -> core::result::Result<Self, D::Error> where D: de::Deserializer<'de> {
                struct Visitor {
                    marker: PhantomData<$ty>,
                }

                impl<'de> de::Visitor<'de> for Visitor
                {
                    type Value = $ty;

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str(concat!("an array of length {}", $len))
                    }

                    #[inline]
                    fn visit_seq<A>(self, mut seq: A) -> core::result::Result<Self::Value, A::Error>
                        where
                            A: de::SeqAccess<'de>,
                    {
                        let mut buf = [0u8; $len];
                        for i in 0..buf.len() {
                            let next = seq.next_element()?;
                            buf[i] = match next {
                                None => return Err(de::Error::invalid_length($len, &self)),
                                Some(val) => val,
                            };
                        }
                        Ok($ty(buf))
                    }
                }

                impl Visitor {
                    fn new() -> Self {
                        Self {
                            marker: PhantomData,
                        }
                    }
                }
                d.deserialize_tuple($len, Visitor::new())
            }
        }

        impl ser::Serialize for $ty {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: Serializer {
                let mut tuple = serializer.serialize_tuple(self.0.len())?;
                for i in 0..self.0.len() {
                    tuple.serialize_element(&self.0[i])?;
                }
                tuple.end()
            }
        }
    }
}

array_impl!(PubKey, 33);

array_impl!(ExtKey, 78);

#[derive(Debug, Serialize, Deserialize)]
pub struct Sha256([u8; 32]);

#[derive(Debug, Serialize, Deserialize)]
pub struct Basepoints {
    revocation: PubKey,
    payment: PubKey,
    htlc: PubKey,
    delayed_payment: PubKey,
}

array_impl!(Signature, 64);
array_impl!(RecoverableSignature, 65);