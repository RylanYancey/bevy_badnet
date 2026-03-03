use bevy::{
    math::{Quat, Vec3},
    transform::components::Transform,
};
use bytes::{Buf, BufMut};
use serde::{Deserialize, Serialize};
use std::{fmt::Debug, marker::PhantomData};

pub trait Encoder<T>: Default + Send + Sync + 'static {
    fn encode(&mut self, item: &T) -> &[u8];
}

pub trait Decoder<T>: Default + Send + Sync + 'static {
    type Error: Debug;

    fn decode(&mut self, buf: &[u8]) -> Result<T, Self::Error>;
}

pub struct JsonCodec<T> {
    buffer: Vec<u8>,
    _marker: PhantomData<T>,
}

impl<T> Default for JsonCodec<T> {
    fn default() -> Self {
        Self {
            buffer: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<T> Encoder<T> for JsonCodec<T>
where
    T: Serialize + Send + Sync + 'static,
{
    fn encode(&mut self, item: &T) -> &[u8] {
        self.buffer.clear();
        // writing to a vec can't fail
        let _ = serde_json::to_writer(&mut self.buffer, item);
        &self.buffer
    }
}

impl<T> Decoder<T> for JsonCodec<T>
where
    T: for<'de> Deserialize<'de> + Send + Sync + 'static,
{
    type Error = serde_json::Error;

    fn decode(&mut self, buf: &[u8]) -> Result<T, Self::Error> {
        serde_json::from_slice(buf)
    }
}

#[cfg(feature = "cbor")]
pub use cbor::CborCodec;

#[cfg(feature = "cbor")]
mod cbor {
    use super::*;
    use bytes::Buf;
    use std::io;

    pub struct CborCodec<T> {
        buffer: Vec<u8>,
        _marker: PhantomData<T>,
    }

    impl<T> Default for CborCodec<T> {
        fn default() -> Self {
            Self {
                buffer: Vec::new(),
                _marker: PhantomData,
            }
        }
    }

    impl<T> Encoder<T> for CborCodec<T>
    where
        T: Serialize,
    {
        fn encode(&mut self, item: &T) -> &[u8] {
            self.buffer.clear();
            // writing to a vec can't fail
            let _ = ciborium::into_writer(item, &mut self.buffer);
            &self.buffer
        }
    }

    impl<T> Decoder<T> for CborCodec<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        type Error = ciborium::de::Error<io::Error>;

        fn decode(&mut self, buf: &[u8]) -> Result<T, Self::Error> {
            ciborium::from_reader(buf.reader())
        }
    }

    impl<T> Codec<T> for CborCodec<T> where
        T: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static
    {
    }
}

#[derive(Default, Clone)]
pub struct TransformCodec {
    buffer: Vec<u8>,
}

impl Encoder<Transform> for TransformCodec {
    fn encode(&mut self, item: &Transform) -> &[u8] {
        self.buffer.clear();

        for v in item.translation.to_array() {
            self.buffer.put_f32_le(v);
        }

        for v in item.rotation.to_array() {
            self.buffer.put_f32_le(v);
        }

        for v in item.scale.to_array() {
            self.buffer.put_f32_le(v);
        }

        &self.buffer
    }
}

impl Decoder<Transform> for TransformCodec {
    type Error = ();

    fn decode(&mut self, mut buf: &[u8]) -> Result<Transform, Self::Error> {
        let mut nums = [0.0f32; 10];
        for i in 0..10 {
            if let Ok(v) = buf.try_get_f32_le() {
                nums[i] = v;
            } else {
                break;
            }
        }

        Ok(Transform {
            translation: Vec3::from_array([nums[0], nums[1], nums[2]]),
            rotation: Quat::from_array([nums[3], nums[4], nums[5], nums[6]]),
            scale: Vec3::from_array([nums[7], nums[8], nums[9]]),
        })
    }
}
