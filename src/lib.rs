#![allow(non_snake_case)]
//! Coherence turbulence box generation using the Mann turbulence model.
//!
//! `Rustmann` provides a computationally efficient module for generating Mann
//! turbulence boxes for wind turbine simulations. `Rustmann` is designed to be
//! called from Python, however the underlying functions are also available in
//! rust.
mod python_interface;
mod tensors;
mod tests;
mod utilities;

pub use self::tensors::Tensors;
pub use self::utilities::Utilities;

use ndarray::parallel::prelude::*;
use ndarray::prelude::*;
use ndarray::Zip;
use ndarray_linalg::Norm;
use ndrustfft::Complex;
use numpy::c64;
use std::f64::consts::PI;
use tensors::Tensors::{Sheared, ShearedSinc, TensorGenerator};


pub fn stencilate_par(
    L: f64,
    gamma: f64,
    Lx: f64,
    Ly: f64,
    Lz: f64,
    Nx: usize,
    Ny: usize,
    Nz: usize,
) -> Array5<f64> {
    let mut stencil: Array5<f64> = Array5::zeros((Nx, Ny, Nz / 2 + 1, 3, 3));
    let (Kx, Ky, Kz): (Array1<f64>, Array1<f64>, Array1<f64>) =
        Utilities::freq_components(Lx, Ly, Lz, Nx, Ny, Nz);
    let tensor_gen = Sheared::from_params(1.0, L, gamma);
    stencil
        .outer_iter_mut()
        .into_par_iter()
        .enumerate()
        .for_each(|(i, mut slice)| {
            for (j, mut column) in slice.outer_iter_mut().enumerate() {
                for (k, mut component) in column.outer_iter_mut().enumerate() {
                    let K = &[Kx[i], Ky[j], Kz[k]];
                    component.assign(&tensor_gen.decomp(K));
                }
            }
        });
    stencil
}

pub fn stencilate_sinc_par(
    L: f64,
    gamma: f64,
    Lx: f64,
    Ly: f64,
    Lz: f64,
    Nx: usize,
    Ny: usize,
    Nz: usize,
) -> Array5<f64> {
    let mut stencil: Array5<f64> = Array5::zeros((Nx, Ny, Nz / 2 + 1, 3, 3));
    let (Kx, Ky, Kz): (Array1<f64>, Array1<f64>, Array1<f64>) =
        Utilities::freq_components(Lx, Ly, Lz, Nx, Ny, Nz);
    let tensor_gen_sinc = ShearedSinc::from_params(1.0, L, gamma, Ly, Lz);
    let tensor_gen = Sheared::from_params(1.0, L, gamma);

    stencil
        .outer_iter_mut()
        .into_par_iter()
        .enumerate()
        .for_each(|(i, mut slice)| {
            for (j, mut column) in slice.outer_iter_mut().enumerate() {
                for (k, mut component) in column.outer_iter_mut().enumerate() {
                    let K = &[Kx[i], Ky[j], Kz[k]];
                    if arr1(K).norm_l2() < 3.0 / L {
                        component.assign(&tensor_gen_sinc.decomp(K));
                    } else {
                        component.assign(&tensor_gen.decomp(K));
                    }
                }
            }
        });
    stencil
}

pub fn partial_turbulate_par(
    stencil: &ArrayView5<f64>,
    ae: f64, 
    seed: u64,
    Nx: usize,
    Ny: usize,
    Nz: usize,
    Lx: f64,
    Ly: f64,
    Lz: f64,
) -> (Array3<c64>, Array3<c64>, Array3<c64>) {
    let KVolScaleFac: c64 = Complex::new(
        2.0 * (Nx * Ny * (Nz / 2 + 1)) as f64 * ((8.0 * ae * PI.powi(3)) / (Lx * Ly * Lz)).sqrt(),
        0.0,
    );
    let random: Array4<c64> = Utilities::complex_random_gaussian(seed, Nx, Ny, Nz / 2 + 1);

    let mut UVW_f: Array4<c64> = Array4::zeros((Nx, Ny, (Nz / 2 + 1), 3));

    Zip::from(UVW_f.outer_iter_mut())
        .and(stencil.outer_iter())
        .and(random.outer_iter())
        .par_for_each(|mut UVW_slice, stencil_slice, random_slice| {
            Zip::from(UVW_slice.outer_iter_mut())
                .and(stencil_slice.outer_iter())
                .and(random_slice.outer_iter())
                .par_for_each(|mut UVW_col, stencil_col, random_col| {
                    Zip::from(UVW_col.outer_iter_mut())
                        .and(stencil_col.outer_iter())
                        .and(random_col.outer_iter())
                        .for_each(|mut freq_comp, tensor, n| {
                            let _tensor = tensor.mapv(|elem| c64::new(elem, 0.0));
                            freq_comp.assign(&_tensor.dot(&n));
                            freq_comp *= KVolScaleFac;
                        })
                })
        });
    UVW_f[[0,0,0,0]] = Complex::new(0.0, 0.0);
    UVW_f[[0,0,0,1]] = Complex::new(0.0, 0.0);
    UVW_f[[0,0,0,2]] = Complex::new(0.0, 0.0);
    (
        UVW_f.slice(s![.., .., .., 0]).to_owned(),
        UVW_f.slice(s![.., .., .., 1]).to_owned(),
        UVW_f.slice(s![.., .., .., 2]).to_owned(),
    )
}

pub fn turbulate_par(
    stencil: &ArrayView5<f64>,
    ae: f64,
    seed: u64,
    Nx: usize,
    Ny: usize,
    Nz: usize,
    Lx: f64,
    Ly: f64,
    Lz: f64,
) -> (Array3<f64>, Array3<f64>, Array3<f64>) {
    let (mut U_f, mut V_f, mut W_f): (Array3<c64>, Array3<c64>, Array3<c64>) =
        partial_turbulate_par(stencil, ae, seed, Nx, Ny, Nz, Lx, Ly, Lz);

    let U: Array3<f64> = Utilities::irfft3d_par(&mut U_f);
    let V: Array3<f64> = Utilities::irfft3d_par(&mut V_f);
    let W: Array3<f64> = Utilities::irfft3d_par(&mut W_f);
    (U, V, W)
}

pub fn stencilate(
    L: f64,
    gamma: f64,
    Lx: f64,
    Ly: f64,
    Lz: f64,
    Nx: usize,
    Ny: usize,
    Nz: usize,
) -> Array5<f64> {
    let mut stencil: Array5<f64> = Array5::zeros((Nx, Ny, Nz / 2 + 1, 3, 3));
    let (Kx, Ky, Kz): (Array1<f64>, Array1<f64>, Array1<f64>) =
        Utilities::freq_components(Lx, Ly, Lz, Nx, Ny, Nz);
    let tensor_gen = Sheared::from_params(1.0, L, gamma);
    stencil
        .outer_iter_mut()
        .into_iter()
        .enumerate()
        .for_each(|(i, mut slice)| {
            for (j, mut column) in slice.outer_iter_mut().enumerate() {
                for (k, mut component) in column.outer_iter_mut().enumerate() {
                    let K = &[Kx[i], Ky[j], Kz[k]];
                    component.assign(&tensor_gen.decomp(K));
                }
            }
        });
    stencil
}

pub fn stencilate_sinc(
    L: f64,
    gamma: f64,
    Lx: f64,
    Ly: f64,
    Lz: f64,
    Nx: usize,
    Ny: usize,
    Nz: usize,
) -> Array5<f64> {
    let mut stencil: Array5<f64> = Array5::zeros((Nx, Ny, Nz / 2 + 1, 3, 3));
    let (Kx, Ky, Kz): (Array1<f64>, Array1<f64>, Array1<f64>) =
        Utilities::freq_components(Lx, Ly, Lz, Nx, Ny, Nz);
    let tensor_gen_sinc = ShearedSinc::from_params(1.0, L, gamma, Ly, Lz);
    let tensor_gen = Sheared::from_params(1.0, L, gamma);

    stencil
        .outer_iter_mut()
        .into_iter()
        .enumerate()
        .for_each(|(i, mut slice)| {
            for (j, mut column) in slice.outer_iter_mut().enumerate() {
                for (k, mut component) in column.outer_iter_mut().enumerate() {
                    let K = &[Kx[i], Ky[j], Kz[k]];
                    if arr1(K).norm_l2() < 3.0 / L {
                        component.assign(&tensor_gen_sinc.decomp(K));
                    } else {
                        component.assign(&tensor_gen.decomp(K));
                    }
                }
            }
        });
    stencil
}

pub fn partial_turbulate(
    stencil: &ArrayView5<f64>,
    ae: f64,
    seed: u64,
    Nx: usize,
    Ny: usize,
    Nz: usize,
    Lx: f64,
    Ly: f64,
    Lz: f64,
) -> (Array3<c64>, Array3<c64>, Array3<c64>) {
    let KVolScaleFac: c64 = Complex::new(
        2.0 * (Nx * Ny * (Nz / 2 + 1)) as f64 * ((8.0 * ae * PI.powi(3)) / (Lx * Ly * Lz)).sqrt(),
        0.0,
    );
    let random: Array4<c64> = Utilities::complex_random_gaussian(seed, Nx, Ny, Nz / 2 + 1);

    let mut UVW_f: Array4<c64> = Array4::zeros((Nx, Ny, (Nz / 2 + 1), 3));

    Zip::from(UVW_f.outer_iter_mut())
        .and(stencil.outer_iter())
        .and(random.outer_iter())
        .for_each(|mut UVW_slice, stencil_slice, random_slice| {
            Zip::from(UVW_slice.outer_iter_mut())
                .and(stencil_slice.outer_iter())
                .and(random_slice.outer_iter())
                .for_each(|mut UVW_col, stencil_col, random_col| {
                    Zip::from(UVW_col.outer_iter_mut())
                        .and(stencil_col.outer_iter())
                        .and(random_col.outer_iter())
                        .for_each(|mut freq_comp, tensor, n| {
                            let _tensor = tensor.mapv(|elem| c64::new(elem, 0.0));
                            freq_comp.assign(&_tensor.dot(&n));
                            freq_comp *= KVolScaleFac;
                        })
                })
        });
    UVW_f[[0,0,0,0]] = Complex::new(0.0, 0.0);
    UVW_f[[0,0,0,1]] = Complex::new(0.0, 0.0);
    UVW_f[[0,0,0,2]] = Complex::new(0.0, 0.0);
    (
        UVW_f.slice(s![.., .., .., 0]).to_owned(),
        UVW_f.slice(s![.., .., .., 1]).to_owned(),
        UVW_f.slice(s![.., .., .., 2]).to_owned(),
    )
}

pub fn turbulate(
    stencil: &ArrayView5<f64>,
    ae: f64,
    seed: u64,
    Nx: usize,
    Ny: usize,
    Nz: usize,
    Lx: f64,
    Ly: f64,
    Lz: f64,
) -> (Array3<f64>, Array3<f64>, Array3<f64>) {
    let (mut U_f, mut V_f, mut W_f): (Array3<c64>, Array3<c64>, Array3<c64>) =
        partial_turbulate(stencil, ae, seed, Nx, Ny, Nz, Lx, Ly, Lz);

    let U: Array3<f64> = Utilities::irfft3d(&mut U_f);
    let V: Array3<f64> = Utilities::irfft3d(&mut V_f);
    let W: Array3<f64> = Utilities::irfft3d(&mut W_f);
    (U, V, W)
}

pub fn partial_turbulate_unit(
    stencil: &ArrayView5<f64>,
    ae: f64,
    seed: u64,
    Nx: usize,
    Ny: usize,
    Nz: usize,
    Lx: f64,
    Ly: f64,
    Lz: f64,
) -> (Array3<c64>, Array3<c64>, Array3<c64>) {
    let KVolScaleFac: c64 = Complex::new(
        2.0 * (Nx * Ny * (Nz / 2 + 1)) as f64 * ((8.0 * ae * PI.powi(3)) / (Lx * Ly * Lz)).sqrt(),
        0.0,
    );
    let random: Array4<c64> = Utilities::complex_random_unit(seed, Nx, Ny, Nz / 2 + 1);

    let mut UVW_f: Array4<c64> = Array4::zeros((Nx, Ny, (Nz / 2 + 1), 3));

    Zip::from(UVW_f.outer_iter_mut())
        .and(stencil.outer_iter())
        .and(random.outer_iter())
        .for_each(|mut UVW_slice, stencil_slice, random_slice| {
            Zip::from(UVW_slice.outer_iter_mut())
                .and(stencil_slice.outer_iter())
                .and(random_slice.outer_iter())
                .for_each(|mut UVW_col, stencil_col, random_col| {
                    Zip::from(UVW_col.outer_iter_mut())
                        .and(stencil_col.outer_iter())
                        .and(random_col.outer_iter())
                        .for_each(|mut freq_comp, tensor, n| {
                            let _tensor = tensor.mapv(|elem| c64::new(elem, 0.0));
                            freq_comp.assign(&_tensor.dot(&n));
                            freq_comp *= KVolScaleFac;
                        })
                })
        });
    UVW_f[[0,0,0,0]] = Complex::new(0.0, 0.0);
    UVW_f[[0,0,0,1]] = Complex::new(0.0, 0.0);
    UVW_f[[0,0,0,2]] = Complex::new(0.0, 0.0);
    (
        UVW_f.slice(s![.., .., .., 0]).to_owned(),
        UVW_f.slice(s![.., .., .., 1]).to_owned(),
        UVW_f.slice(s![.., .., .., 2]).to_owned(),
    )
}

pub fn turbulate_unit(
    stencil: &ArrayView5<f64>,
    ae: f64,
    seed: u64,
    Nx: usize,
    Ny: usize,
    Nz: usize,
    Lx: f64,
    Ly: f64,
    Lz: f64,
) -> (Array3<f64>, Array3<f64>, Array3<f64>) {
    let (mut U_f, mut V_f, mut W_f): (Array3<c64>, Array3<c64>, Array3<c64>) =
        partial_turbulate_unit(stencil, ae, seed, Nx, Ny, Nz, Lx, Ly, Lz);

    let U: Array3<f64> = Utilities::irfft3d(&mut U_f);
    let V: Array3<f64> = Utilities::irfft3d(&mut V_f);
    let W: Array3<f64> = Utilities::irfft3d(&mut W_f);
    (U, V, W)
}

