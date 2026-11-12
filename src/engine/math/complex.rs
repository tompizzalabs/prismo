use std::ops::{Add, Mul, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Complex64 {
    pub real_part: f64,
    pub imaginary_part: f64,
}

impl Complex64 {
    pub const ZERO: Complex64 = Complex64 {
        real_part: 0.0,
        imaginary_part: 0.0,
    };

    pub const ONE: Complex64 = Complex64 {
        real_part: 1.0,
        imaginary_part: 0.0,
    };

    #[inline]
    pub fn new(real_part: f64, imaginary_part: f64) -> Self {
        Complex64 {
            real_part,
            imaginary_part,
        }
    }

    #[allow(unused)]
    #[inline]
    pub fn from_real(real_part: f64) -> Self {
        Complex64 {
            real_part,
            imaginary_part: 0.0,
        }
    }

    #[inline]
    pub fn magnitude_squared(&self) -> f64 {
        self.real_part * self.real_part + self.imaginary_part * self.imaginary_part
    }

    #[allow(unused)]
    #[inline]
    pub fn conjugate(&self) -> Self {
        Complex64::new(self.real_part, -self.imaginary_part)
    }

    #[inline]
    pub fn exponential(&self) -> Self {
        let scale = self.real_part.exp();
        let (sine_part, cosine_part) = self.imaginary_part.sin_cos();
        Complex64::new(scale * cosine_part, scale * sine_part)
    }

    #[inline]
    pub fn inverse(&self) -> Self {
        let denominator = self.magnitude_squared();
        Complex64::new(
            self.real_part / denominator,
            -self.imaginary_part / denominator,
        )
    }

    #[inline]
    pub fn scaled(&self, factor: f64) -> Self {
        Complex64::new(self.real_part * factor, self.imaginary_part * factor)
    }
}

impl Add for Complex64 {
    type Output = Complex64;

    #[inline]
    fn add(self, other: Complex64) -> Complex64 {
        Complex64::new(
            self.real_part + other.real_part,
            self.imaginary_part + other.imaginary_part,
        )
    }
}

impl Sub for Complex64 {
    type Output = Complex64;

    #[inline]
    fn sub(self, other: Complex64) -> Complex64 {
        Complex64::new(
            self.real_part - other.real_part,
            self.imaginary_part - other.imaginary_part,
        )
    }
}

impl Mul for Complex64 {
    type Output = Complex64;

    #[inline]
    fn mul(self, other: Complex64) -> Complex64 {
        Complex64::new(
            self.real_part * other.real_part - self.imaginary_part * other.imaginary_part,
            self.real_part * other.imaginary_part + self.imaginary_part * other.real_part,
        )
    }
}

impl Mul<f64> for Complex64 {
    type Output = Complex64;

    #[inline]
    fn mul(self, factor: f64) -> Complex64 {
        self.scaled(factor)
    }
}
