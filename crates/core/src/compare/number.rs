//! Exact numeric comparison across Int32, Int64, Double and Decimal128, the way MongoDB orders
//! numbers: by mathematical value, with NaN below every other number (and equal to itself) and
//! `-0 == 0`. Int/Int, Double/Double and Int/Double take allocation-free fast paths; Decimal128
//! values are compared as normalised decimals, and a Double meeting a Decimal128 is expanded to
//! its exact decimal value (a rare path).

use std::cmp::Ordering;

use bson::{Bson, Decimal128};

/// A decimal in canonical form: `digits` has no leading or trailing zeros (zero has none at all)
/// and the value is `digits × 10^exp`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Dec {
    NaN,
    NegInf,
    PosInf,
    Finite {
        neg: bool,
        digits: Vec<u8>,
        exp: i64,
    },
}

impl Dec {
    fn zero() -> Self {
        Self::Finite {
            neg: false,
            digits: Vec::new(),
            exp: 0,
        }
    }

    /// Strips leading and trailing zeros; `-0` becomes `0`.
    fn normalized(neg: bool, mut digits: Vec<u8>, mut exp: i64) -> Self {
        let leading = digits.iter().take_while(|d| **d == 0).count();
        digits.drain(..leading);
        if digits.is_empty() {
            return Self::zero();
        }
        while digits.last() == Some(&0) {
            digits.pop();
            exp += 1;
        }
        Self::Finite { neg, digits, exp }
    }

    fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        match text.to_ascii_lowercase().as_str() {
            "nan" | "+nan" | "-nan" | "snan" | "-snan" => return Some(Self::NaN),
            "infinity" | "+infinity" | "inf" | "+inf" => return Some(Self::PosInf),
            "-infinity" | "-inf" => return Some(Self::NegInf),
            _ => {}
        }
        let (neg, rest) = match text.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let (mantissa, exp_part) = match rest.split_once(['e', 'E']) {
            Some((m, e)) => (m, e.parse::<i64>().ok()?),
            None => (rest, 0),
        };
        let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        if int_part.is_empty() && frac_part.is_empty() {
            return None;
        }
        if !int_part
            .bytes()
            .chain(frac_part.bytes())
            .all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let digits: Vec<u8> = int_part
            .bytes()
            .chain(frac_part.bytes())
            .map(|b| b - b'0')
            .collect();
        Some(Self::normalized(
            neg,
            digits,
            exp_part - frac_part.len() as i64,
        ))
    }

    pub(crate) fn of(d: &Decimal128) -> Self {
        // A valid Decimal128 always prints in a form `parse` understands.
        Self::parse(&d.to_string()).unwrap_or(Self::NaN)
    }

    fn from_i64(n: i64) -> Self {
        let digits = n
            .unsigned_abs()
            .to_string()
            .bytes()
            .map(|b| b - b'0')
            .collect();
        Self::normalized(n < 0, digits, 0)
    }

    /// The exact decimal value of a finite double, expanded from its binary form.
    pub(crate) fn from_f64(d: f64) -> Self {
        if d.is_nan() {
            return Self::NaN;
        }
        if d.is_infinite() {
            return if d > 0.0 { Self::PosInf } else { Self::NegInf };
        }
        if d == 0.0 {
            return Self::zero();
        }
        let bits = d.to_bits();
        let neg = bits >> 63 == 1;
        let exp_bits = ((bits >> 52) & 0x7ff) as i64;
        let frac = bits & ((1u64 << 52) - 1);
        let (mantissa, e2) = if exp_bits == 0 {
            (frac, -1074)
        } else {
            (frac | (1u64 << 52), exp_bits - 1075)
        };
        let mut digits_le = digits_le(mantissa);
        let exp10 = if e2 >= 0 {
            for _ in 0..e2 {
                mul_small(&mut digits_le, 2);
            }
            0
        } else {
            for _ in 0..(-e2) {
                mul_small(&mut digits_le, 5);
            }
            e2
        };
        digits_le.reverse();
        Self::normalized(neg, digits_le, exp10)
    }

    /// The value as an `i64`, only when it is integral and in range.
    pub(crate) fn to_i64(&self) -> Option<i64> {
        let Self::Finite { neg, digits, exp } = self else {
            return None;
        };
        if digits.is_empty() {
            return Some(0);
        }
        if *exp < 0 {
            return None;
        }
        let total = digits.len() as i64 + exp;
        if total > 19 {
            return None;
        }
        let mut magnitude: u128 = 0;
        for d in digits {
            magnitude = magnitude * 10 + u128::from(*d);
        }
        for _ in 0..*exp {
            magnitude *= 10;
        }
        if *neg {
            (magnitude <= 1u128 << 63).then(|| (-(magnitude as i128)) as i64)
        } else {
            i64::try_from(magnitude).ok()
        }
    }

    /// A canonical `Decimal128` for a finite value (equal values give identical bytes).
    pub(crate) fn to_decimal128(&self) -> Option<Decimal128> {
        let (neg, digits, exp) = match self {
            Self::Finite { neg, digits, exp } => (*neg, digits, *exp),
            _ => return None,
        };
        let text = if digits.is_empty() {
            "0".to_string()
        } else {
            let ds: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
            format!("{}{ds}E{exp}", if neg { "-" } else { "" })
        };
        text.parse().ok()
    }

    fn class(&self) -> u8 {
        match self {
            Self::NaN => 0,
            Self::NegInf => 1,
            Self::Finite { .. } => 2,
            Self::PosInf => 3,
        }
    }
}

fn digits_le(mut n: u64) -> Vec<u8> {
    if n == 0 {
        return vec![0];
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push((n % 10) as u8);
        n /= 10;
    }
    out
}

/// Multiplies a little-endian decimal digit vector by a small factor.
fn mul_small(digits: &mut Vec<u8>, factor: u32) {
    let mut carry = 0u32;
    for d in digits.iter_mut() {
        let v = u32::from(*d) * factor + carry;
        *d = (v % 10) as u8;
        carry = v / 10;
    }
    while carry > 0 {
        digits.push((carry % 10) as u8);
        carry /= 10;
    }
}

impl Ord for Dec {
    fn cmp(&self, other: &Self) -> Ordering {
        let (
            Self::Finite {
                neg: n1,
                digits: d1,
                exp: e1,
            },
            Self::Finite {
                neg: n2,
                digits: d2,
                exp: e2,
            },
        ) = (self, other)
        else {
            return self.class().cmp(&other.class());
        };
        let sign = |neg: bool, digits: &[u8]| -> i8 {
            if digits.is_empty() {
                0
            } else if neg {
                -1
            } else {
                1
            }
        };
        let (s1, s2) = (sign(*n1, d1), sign(*n2, d2));
        if s1 != s2 {
            return s1.cmp(&s2);
        }
        if s1 == 0 {
            return Ordering::Equal;
        }
        let adjusted = |digits: &[u8], exp: i64| exp + digits.len() as i64 - 1;
        let magnitude = adjusted(d1, *e1)
            .cmp(&adjusted(d2, *e2))
            .then_with(|| d1.cmp(d2));
        if s1 > 0 {
            magnitude
        } else {
            magnitude.reverse()
        }
    }
}

impl PartialOrd for Dec {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

enum Num<'a> {
    Int(i64),
    Double(f64),
    Decimal(&'a Decimal128),
}

fn num(b: &Bson) -> Option<Num<'_>> {
    match b {
        Bson::Int32(n) => Some(Num::Int(i64::from(*n))),
        Bson::Int64(n) => Some(Num::Int(*n)),
        Bson::Double(d) => Some(Num::Double(*d)),
        Bson::Decimal128(d) => Some(Num::Decimal(d)),
        _ => None,
    }
}

fn cmp_f64(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

/// Exact comparison of an `i64` with a double, without converting the integer to a float.
fn cmp_i64_f64(n: i64, d: f64) -> Ordering {
    if d.is_nan() {
        return Ordering::Greater;
    }
    if d >= 9_223_372_036_854_775_808.0 {
        return Ordering::Less;
    }
    if d < -9_223_372_036_854_775_808.0 {
        return Ordering::Greater;
    }
    let truncated = d.trunc();
    // In range and integral, so the cast is exact.
    match n.cmp(&(truncated as i64)) {
        Ordering::Equal => {
            let fraction = d - truncated;
            if fraction > 0.0 {
                Ordering::Less
            } else if fraction < 0.0 {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        other => other,
    }
}

/// Compares two numeric `Bson` values. Callers pass numbers only; anything else compares equal.
pub(crate) fn compare_numbers(a: &Bson, b: &Bson) -> Ordering {
    use Num::{Decimal, Double, Int};
    let (Some(x), Some(y)) = (num(a), num(b)) else {
        return Ordering::Equal;
    };
    match (x, y) {
        (Int(p), Int(q)) => p.cmp(&q),
        (Double(p), Double(q)) => cmp_f64(p, q),
        (Int(n), Double(d)) => cmp_i64_f64(n, d),
        (Double(d), Int(n)) => cmp_i64_f64(n, d).reverse(),
        (Decimal(p), Decimal(q)) => Dec::of(p).cmp(&Dec::of(q)),
        (Decimal(p), Int(n)) => Dec::of(p).cmp(&Dec::from_i64(n)),
        (Int(n), Decimal(q)) => Dec::from_i64(n).cmp(&Dec::of(q)),
        (Decimal(p), Double(d)) => Dec::of(p).cmp(&Dec::from_f64(d)),
        (Double(d), Decimal(q)) => Dec::from_f64(d).cmp(&Dec::of(q)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::Decimal128;
    use std::cmp::Ordering::{Equal, Greater, Less};
    use std::str::FromStr;

    fn dec(s: &str) -> Bson {
        Bson::Decimal128(Decimal128::from_str(s).expect("valid decimal"))
    }

    #[test]
    fn equal_values_of_different_types_compare_equal() {
        let one = [
            Bson::Int32(1),
            Bson::Int64(1),
            Bson::Double(1.0),
            dec("1"),
            dec("1.00"),
            dec("0.1E+1"),
        ];
        for a in &one {
            for b in &one {
                assert_eq!(compare_numbers(a, b), Equal, "{a:?} vs {b:?}");
            }
        }
        assert_eq!(compare_numbers(&dec("1E+3"), &Bson::Int32(1000)), Equal);
        assert_eq!(compare_numbers(&dec("-0"), &Bson::Int32(0)), Equal);
        assert_eq!(
            compare_numbers(&Bson::Double(-0.0), &Bson::Double(0.0)),
            Equal
        );
        assert_eq!(compare_numbers(&Bson::Double(-0.0), &Bson::Int64(0)), Equal);
    }

    #[test]
    fn int64_is_compared_exactly_above_two_to_the_53() {
        let two53: i64 = 1 << 53;
        assert_eq!(
            compare_numbers(&Bson::Int64(two53 + 1), &Bson::Double(two53 as f64)),
            Greater
        );
        assert_eq!(
            compare_numbers(&Bson::Int64(two53), &Bson::Double(two53 as f64)),
            Equal
        );
        assert_eq!(
            compare_numbers(
                &Bson::Int64(i64::MAX),
                &Bson::Double(9_223_372_036_854_775_808.0)
            ),
            Less
        );
        assert_eq!(
            compare_numbers(
                &Bson::Int64(i64::MIN),
                &Bson::Double(-9_223_372_036_854_775_808.0)
            ),
            Equal
        );
        assert_eq!(
            compare_numbers(&Bson::Int64(i64::MIN), &Bson::Double(f64::NEG_INFINITY)),
            Greater
        );
        assert_eq!(compare_numbers(&Bson::Int64(0), &Bson::Double(0.5)), Less);
        assert_eq!(
            compare_numbers(&Bson::Int64(0), &Bson::Double(-0.5)),
            Greater
        );
    }

    #[test]
    fn nan_is_below_every_number_and_equals_itself() {
        let nan = Bson::Double(f64::NAN);
        assert_eq!(compare_numbers(&nan, &Bson::Double(f64::NAN)), Equal);
        assert_eq!(
            compare_numbers(&nan, &Bson::Double(f64::NEG_INFINITY)),
            Less
        );
        assert_eq!(compare_numbers(&nan, &Bson::Int64(i64::MIN)), Less);
        assert_eq!(compare_numbers(&nan, &dec("NaN")), Equal);
        assert_eq!(compare_numbers(&dec("NaN"), &dec("-Infinity")), Less);
        assert_eq!(compare_numbers(&Bson::Int32(0), &nan), Greater);
    }

    #[test]
    fn decimals_compare_by_exact_value() {
        assert_eq!(compare_numbers(&dec("1.5"), &dec("1.25")), Greater);
        assert_eq!(compare_numbers(&dec("1"), &dec("1.5")), Less);
        assert_eq!(compare_numbers(&dec("-1.5"), &dec("-1.25")), Less);
        assert_eq!(compare_numbers(&dec("-1"), &dec("0")), Less);
        assert_eq!(compare_numbers(&dec("0.001"), &dec("0.0001")), Greater);
        assert_eq!(
            compare_numbers(
                &dec("1E+6144"),
                &dec("9.999999999999999999999999999999999E+6143")
            ),
            Greater
        );
        assert_eq!(compare_numbers(&dec("1E-6176"), &dec("0")), Greater);
        assert_eq!(
            compare_numbers(&dec("Infinity"), &Bson::Int64(i64::MAX)),
            Greater
        );
        assert_eq!(
            compare_numbers(&dec("-Infinity"), &Bson::Int64(i64::MIN)),
            Less
        );
    }

    #[test]
    fn a_double_is_compared_to_a_decimal_by_its_exact_binary_value() {
        // 0.1 as a double is slightly MORE than one tenth, so the decimal 0.1 is smaller.
        assert_eq!(compare_numbers(&dec("0.1"), &Bson::Double(0.1)), Less);
        assert_eq!(compare_numbers(&Bson::Double(0.1), &dec("0.1")), Greater);
        // 0.5 is exactly representable.
        assert_eq!(compare_numbers(&dec("0.5"), &Bson::Double(0.5)), Equal);
        assert_eq!(compare_numbers(&Bson::Double(1e30), &dec("1E+30")), Greater);
        assert_eq!(compare_numbers(&Bson::Double(5e-324), &dec("0")), Greater);
    }

    #[test]
    fn dec_converts_to_i64_only_when_integral_and_in_range() {
        assert_eq!(
            Dec::of(&Decimal128::from_str("12").unwrap()).to_i64(),
            Some(12)
        );
        assert_eq!(
            Dec::of(&Decimal128::from_str("1.20E+1").unwrap()).to_i64(),
            Some(12)
        );
        assert_eq!(
            Dec::of(&Decimal128::from_str("-0").unwrap()).to_i64(),
            Some(0)
        );
        assert_eq!(
            Dec::of(&Decimal128::from_str("1.5").unwrap()).to_i64(),
            None
        );
        assert_eq!(
            Dec::of(&Decimal128::from_str("9223372036854775807").unwrap()).to_i64(),
            Some(i64::MAX)
        );
        assert_eq!(
            Dec::of(&Decimal128::from_str("9223372036854775808").unwrap()).to_i64(),
            None
        );
        assert_eq!(
            Dec::of(&Decimal128::from_str("-9223372036854775808").unwrap()).to_i64(),
            Some(i64::MIN)
        );
        assert_eq!(
            Dec::of(&Decimal128::from_str("NaN").unwrap()).to_i64(),
            None
        );
    }

    #[test]
    fn dec_round_trips_through_a_canonical_decimal128() {
        for s in [
            "1.5",
            "1.50",
            "-0.001",
            "1E+3",
            "123456789012345678901234567890",
            "1E-6176",
        ] {
            let d = Dec::of(&Decimal128::from_str(s).unwrap());
            let back = d.to_decimal128().expect("finite values convert");
            assert_eq!(Dec::of(&back), d, "{s}");
        }
        // Equal values written differently normalise to the same Decimal128 bytes.
        let a = Dec::of(&Decimal128::from_str("1.5").unwrap())
            .to_decimal128()
            .unwrap();
        let b = Dec::of(&Decimal128::from_str("1.50").unwrap())
            .to_decimal128()
            .unwrap();
        assert_eq!(a.bytes(), b.bytes());
    }
}
