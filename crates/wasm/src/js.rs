//! Building and reading plain JS values.

use js_sys::{Float64Array, Object, Reflect};
use tradoor_core::Candle;
use wasm_bindgen::JsValue;

pub fn obj() -> Object {
    Object::new()
}

pub fn set(o: &Object, key: &str, value: impl Into<JsValue>) {
    let _ = Reflect::set(o, &JsValue::from_str(key), &value.into());
}

pub fn get(o: &JsValue, key: &str) -> Option<JsValue> {
    Reflect::get(o, &JsValue::from_str(key)).ok().filter(|v| !v.is_undefined() && !v.is_null())
}

pub fn opt(v: Option<f64>) -> JsValue {
    v.map_or(JsValue::NULL, JsValue::from_f64)
}

pub fn point(time: f64, price: f64) -> Object {
    let o = obj();
    set(&o, "time", time);
    set(&o, "price", price);
    o
}

pub fn candles(time: &[f64], open: &[f64], high: &[f64], low: &[f64], close: &[f64], volume: &[f64]) -> Vec<Candle> {
    let n = time.len().min(open.len()).min(high.len()).min(low.len()).min(close.len()).min(volume.len());
    (0..n).map(|i| Candle { open_time: time[i] as i64, open: open[i], high: high[i], low: low[i], close: close[i], volume: volume[i], quote_volume: None, taker_buy: None }).collect()
}

/// What market buyers took in each bar, as `columns` writes it, onto candles.
pub fn with_taker(mut candles: Vec<Candle>, buy: &[f64]) -> Vec<Candle> {
    for (c, b) in candles.iter_mut().zip(buy) {
        c.taker_buy = (*b >= 0.0).then_some(*b);
    }
    candles
}

pub fn columns(candles: &[Candle]) -> Object {
    let col = |f: fn(&Candle) -> f64| Float64Array::from(candles.iter().map(f).collect::<Vec<_>>().as_slice());
    let o = obj();
    set(&o, "time", col(|c| c.open_time as f64));
    set(&o, "open", col(|c| c.open));
    set(&o, "high", col(|c| c.high));
    set(&o, "low", col(|c| c.low));
    set(&o, "close", col(|c| c.close));
    set(&o, "volume", col(|c| c.volume));
    // -1 where the exchange does not say.
    set(&o, "buy", col(|c| c.taker_buy.unwrap_or(-1.0)));
    o
}
