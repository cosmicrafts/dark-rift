use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Sub, SubAssign};

/// A high-performance, zero-allocation 16-byte scientific float representation for colossal numbers (10^0 to 10^10000+).
/// Format: mantissa * 10^exponent, where 1.0 <= mantissa < 10.0 (or 0.0).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Serialize, Deserialize)]
pub struct MassiveNumber {
    pub mantissa: f64,
    pub exponent: i64,
}

impl MassiveNumber {
    pub const ZERO: Self = Self {
        mantissa: 0.0,
        exponent: 0,
    };
    pub const ONE: Self = Self {
        mantissa: 1.0,
        exponent: 0,
    };

    #[inline]
    pub fn new(mantissa: f64, exponent: i64) -> Self {
        let mut s = Self { mantissa, exponent };
        s.normalize();
        s
    }

    #[inline]
    pub fn from_f64(v: f64) -> Self {
        if v.abs() < 1e-15 || !v.is_finite() {
            return Self::ZERO;
        }
        let exp = libm::floor(libm::log10(v.abs())) as i64;
        let mant = v / libm::pow(10.0, exp as f64);
        Self::new(mant, exp)
    }

    #[inline]
    pub fn from_u64(v: u64) -> Self {
        Self::from_f64(v as f64)
    }

    #[inline]
    pub fn is_zero(&self) -> bool {
        self.mantissa.abs() < 1e-12
    }

    #[inline]
    pub fn to_f64(&self) -> f64 {
        if self.is_zero() {
            0.0
        } else if self.exponent > 308 {
            f64::INFINITY
        } else if self.exponent < -308 {
            0.0
        } else {
            self.mantissa * libm::pow(10.0, self.exponent as f64)
        }
    }

    #[inline]
    pub fn normalize(&mut self) {
        if self.mantissa.abs() < 1e-12 || !self.mantissa.is_finite() {
            self.mantissa = 0.0;
            self.exponent = 0;
            return;
        }

        let abs_m = self.mantissa.abs();
        if abs_m >= 10.0 {
            let shift = libm::floor(libm::log10(abs_m)) as i64;
            self.mantissa /= libm::pow(10.0, shift as f64);
            self.exponent += shift;
        } else if abs_m < 1.0 {
            let shift = libm::floor(libm::log10(abs_m)) as i64; // negative
            self.mantissa /= libm::pow(10.0, shift as f64);
            self.exponent += shift;
        }
    }

    #[inline]
    pub fn mul_f64(&self, scalar: f64) -> Self {
        if self.is_zero() || scalar.abs() < 1e-15 {
            Self::ZERO
        } else {
            Self::new(self.mantissa * scalar, self.exponent)
        }
    }

    #[inline]
    pub fn div_f64(&self, scalar: f64) -> Self {
        if scalar.abs() < 1e-15 {
            Self::ZERO
        } else {
            Self::new(self.mantissa / scalar, self.exponent)
        }
    }

    #[inline]
    pub fn pow_f64(&self, p: f64) -> Self {
        if self.is_zero() {
            return Self::ZERO;
        }
        if p.abs() < 1e-15 {
            return Self::ONE;
        }
        // log10(x^p) = p * (log10(m) + exp)
        let total_log = p * (libm::log10(self.mantissa) + self.exponent as f64);
        let new_exp = libm::floor(total_log) as i64;
        let new_mant = libm::pow(10.0, total_log - new_exp as f64);
        Self::new(new_mant, new_exp)
    }

    #[inline]
    pub fn sqrt(&self) -> Self {
        self.pow_f64(0.5)
    }

    /// Formats number into concise compact idle notation (e.g. 1.23 K, 4.56 M, 9.87 B, 1.23e45).
    pub fn format_compact(&self) -> String {
        if self.is_zero() {
            return "0".to_string();
        }
        if self.exponent < 3 {
            let v = self.to_f64();
            if v.fract().abs() < 0.01 {
                return format!("{:.0}", v);
            } else {
                return format!("{:.2}", v);
            }
        }

        const SUFFIXES: &[&str] = &[
            "", "K", "M", "B", "T", "Qa", "Qi", "Sx", "Sp", "Oc", "No", "Dc", "Ud", "Dd", "Td",
            "Qad", "Qid", "Sxd", "Spd", "Ocd", "Nod", "Vg",
        ];

        let tier = (self.exponent / 3) as usize;
        if tier < SUFFIXES.len() {
            let rem = (self.exponent % 3) as f64;
            let display_val = self.mantissa * libm::pow(10.0, rem);
            format!("{:.2} {}", display_val, SUFFIXES[tier])
        } else {
            format!("{:.2}e{}", self.mantissa, self.exponent)
        }
    }

    /// Formats mass according to astrophysical scale (g -> kg -> t -> Yg -> M_Earth -> M_Sun -> M_MilkyWay).
    pub fn format_mass_kg(&self) -> String {
        if self.is_zero() {
            return "0 kg".to_string();
        }
        // 1 Solar Mass (M☉) = 1.989e30 kg
        // 1 Earth Mass (M⊕) = 5.972e24 kg
        // 1 Lunar Mass = 7.342e22 kg
        // 1 Galaxy Mass (Milky Way) ≈ 1.5e42 kg
        // Observable Universe ≈ 1.5e53 kg
        if self.exponent >= 50 {
            let uni = *self / Self::from_f64(1.5e53);
            format!("{} U_mass", uni.format_compact())
        } else if self.exponent >= 40 {
            let mw = *self / Self::from_f64(1.5e42);
            format!("{} M_gal", mw.format_compact())
        } else if self.exponent >= 28 {
            let suns = *self / Self::from_f64(1.989e30);
            format!("{} M☉", suns.format_compact())
        } else if self.exponent >= 23 {
            let earths = *self / Self::from_f64(5.972e24);
            format!("{} M⊕", earths.format_compact())
        } else {
            format!("{} kg", self.format_compact())
        }
    }
}

impl PartialEq for MassiveNumber {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        if self.is_zero() && other.is_zero() {
            true
        } else {
            self.exponent == other.exponent && (self.mantissa - other.mantissa).abs() < 1e-6
        }
    }
}

impl PartialOrd for MassiveNumber {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.is_zero() && other.is_zero() {
            return Some(Ordering::Equal);
        }
        if self.is_zero() {
            return if other.mantissa > 0.0 {
                Some(Ordering::Less)
            } else {
                Some(Ordering::Greater)
            };
        }
        if other.is_zero() {
            return if self.mantissa > 0.0 {
                Some(Ordering::Greater)
            } else {
                Some(Ordering::Less)
            };
        }

        if self.exponent != other.exponent {
            self.exponent.partial_cmp(&other.exponent)
        } else {
            self.mantissa.partial_cmp(&other.mantissa)
        }
    }
}

impl Add for MassiveNumber {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        if self.is_zero() {
            return rhs;
        }
        if rhs.is_zero() {
            return self;
        }

        let diff = self.exponent - rhs.exponent;
        if diff > 15 {
            self
        } else if diff < -15 {
            rhs
        } else if diff >= 0 {
            let m = self.mantissa + rhs.mantissa * libm::pow(10.0, -diff as f64);
            Self::new(m, self.exponent)
        } else {
            let m = self.mantissa * libm::pow(10.0, diff as f64) + rhs.mantissa;
            Self::new(m, rhs.exponent)
        }
    }
}

impl Sub for MassiveNumber {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        if rhs.is_zero() {
            return self;
        }
        if self.is_zero() {
            return Self::new(-rhs.mantissa, rhs.exponent);
        }

        let diff = self.exponent - rhs.exponent;
        if diff > 15 {
            self
        } else if diff < -15 {
            Self::new(-rhs.mantissa, rhs.exponent)
        } else if diff >= 0 {
            let m = self.mantissa - rhs.mantissa * libm::pow(10.0, -diff as f64);
            Self::new(m, self.exponent)
        } else {
            let m = self.mantissa * libm::pow(10.0, diff as f64) - rhs.mantissa;
            Self::new(m, rhs.exponent)
        }
    }
}

impl Mul for MassiveNumber {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        if self.is_zero() || rhs.is_zero() {
            Self::ZERO
        } else {
            Self::new(self.mantissa * rhs.mantissa, self.exponent + rhs.exponent)
        }
    }
}

impl Div for MassiveNumber {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self::Output {
        if self.is_zero() || rhs.is_zero() {
            Self::ZERO
        } else {
            Self::new(self.mantissa / rhs.mantissa, self.exponent - rhs.exponent)
        }
    }
}

impl AddAssign for MassiveNumber {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl SubAssign for MassiveNumber {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl MulAssign for MassiveNumber {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}

impl DivAssign for MassiveNumber {
    #[inline]
    fn div_assign(&mut self, rhs: Self) {
        *self = *self / rhs;
    }
}

impl fmt::Debug for MassiveNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_compact())
    }
}

impl fmt::Display for MassiveNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_compact())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_massive_number_arithmetic() {
        let a = MassiveNumber::from_f64(100.0);
        let b = MassiveNumber::from_f64(200.0);
        assert_eq!((a + b).to_f64(), 300.0);
        assert_eq!((b - a).to_f64(), 100.0);
        assert_eq!((a * b).to_f64(), 20000.0);
        assert_eq!((b / a).to_f64(), 2.0);

        let big1 = MassiveNumber::new(1.5, 100);
        let big2 = MassiveNumber::new(2.0, 100);
        let sum = big1 + big2;
        assert_eq!(sum.mantissa, 3.5);
        assert_eq!(sum.exponent, 100);

        let prod = big1 * big2;
        assert_eq!(prod.mantissa, 3.0);
        assert_eq!(prod.exponent, 200);
    }
}
