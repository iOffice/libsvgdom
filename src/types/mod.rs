// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! This module contains submodules which represents SVG value types.

pub use self::transform::Transform;
pub use self::color::Color;
pub use self::length::Length;

pub use svgparser::{LengthUnit};

pub use super::attribute::NumberList;
pub use super::attribute::LengthList;

pub mod path;
mod color;
mod transform;
mod length;
mod number;
