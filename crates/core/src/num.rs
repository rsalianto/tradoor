//! `clamp` for floats, without its panic: the message it would print formats two
//! floats, which alone keeps all of the standard float printer in the wasm.

pub trait Within: Sized {
    fn within(self, low: Self, high: Self) -> Self;
}

macro_rules! within {
    ($t:ty) => {
        impl Within for $t {
            #[inline]
            fn within(self, low: Self, high: Self) -> Self {
                if self < low { low } else if self > high { high } else { self }
            }
        }
    };
}
within!(f32);
within!(f64);
