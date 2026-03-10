use indicatif::{ProgressBar, ProgressStyle};
use opencv::{
    core,
    imgcodecs,
    prelude::*,
    videoio::{self, VideoCapture, VideoCaptureTrait},
    Result,
};
use rand::Rng;
use rayon::prelude::*;
use std::env;
use std::sync::{mpsc, Arc};
use std::thread;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    
    // Parse arguments
    let input_path = if args.len() > 1 { args[1].clone() } else { "input.mp4".to_string() };
    let output_path = if args.len() > 2 { args[2].clone() } else { "output.png".to_string() };
    let method = if args.len() > 3 { args[3].to_lowercase() } else { "mean".to_string() };

    println!("Opening video: {}", input_path);
    println!("Output will be saved to: {}", output_path);
    println!("Selected method: {}", method);

    // Fetch initial metadata
    let mut cap = VideoCapture::default()?;
    if !cap.open_file(&input_path, videoio::CAP_ANY)? || !cap.is_opened()? {
        panic!("Error: Unable to open video file. Please check the path.");
    }

    let width = cap.get(videoio::CAP_PROP_FRAME_WIDTH)? as usize;
    let height = cap.get(videoio::CAP_PROP_FRAME_HEIGHT)? as usize;
    let total_frames = cap.get(videoio::CAP_PROP_FRAME_COUNT)? as u64;
    
    println!("Video Resolution: {}x{}", width, height);
    println!("Total Frames: {}", total_frames);
    
    // Close the initial capture so the individual methods can open it cleanly
    cap.release()?;

    // Dispatch to the selected method
    match method.as_str() {
        "mean" | "average" => process_mean(&input_path, &output_path, width, height, total_frames)?,
        "median" => process_median(&input_path, &output_path, width, height, total_frames)?,
        "random" => process_random(&input_path, &output_path, width, height, total_frames)?,
        _ => {
            println!("Unknown method '{}'. Falling back to 'mean'.", method);
            process_mean(&input_path, &output_path, width, height, total_frames)?;
        }
    }

    Ok(())
}

// ==========================================
// 1. MEAN (AVERAGE) WITH GAMMA CORRECTION
// ==========================================
fn process_mean(input_path: &str, output_path: &str, width: usize, height: usize, total_frames: u64) -> Result<()> {
    let num_elements = width * height * 3;
    let gamma = 2.2f64;
    
    let mut decode_lut = [0f64; 256];
    for i in 0..256 {
        decode_lut[i] = (i as f64 / 255.0).powf(gamma);
    }

    let pb = ProgressBar::new(total_frames);
    pb.set_style(
        ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} frames ({fps:.1} fps) ETA: {eta}")
        .unwrap()
        .progress_chars("#>-")
    );

    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(30);
    let producer_pb = pb.clone();
    let input_clone = input_path.to_string();

    let producer = thread::spawn(move || -> Result<u64> {
        let mut cap = VideoCapture::default()?;
        cap.open_file(&input_clone, videoio::CAP_ANY)?;
        let mut frame_count = 0u64;
        let mut frame = Mat::default();
        let mut continuous_frame = Mat::default();

        loop {
            let success = cap.read(&mut frame)?;
            if !success || frame.empty() { break; }

            if !frame.is_continuous() {
                frame.copy_to(&mut continuous_frame)?;
            } else {
                continuous_frame = frame.clone();
            }

            let bytes = continuous_frame.data_bytes()?.to_vec();
            if tx.send(bytes).is_err() { break; }

            frame_count += 1;
            producer_pb.inc(1);
        }
        
        producer_pb.finish_with_message("Video reading complete.");
        Ok(frame_count)
    });

    let accumulator = rx.into_iter()
        .par_bridge()
        .fold(
            || vec![0f64; num_elements],
            |mut local_acc: Vec<f64>, frame_data: Vec<u8>| {
                for (a, &b) in local_acc.iter_mut().zip(&frame_data) {
                    *a += decode_lut[b as usize];
                }
                local_acc
            }
        )
        .reduce(
            || vec![0f64; num_elements],
            |mut acc1, acc2| {
                for (a, b) in acc1.iter_mut().zip(acc2) {
                    *a += b;
                }
                acc1
            }
        );

    let frame_count = producer.join().expect("Producer thread panicked")?;
    if frame_count == 0 { panic!("Error: No frames were read from the video."); }

    println!("\nGenerating final gamma-corrected average image...");
    let mut final_mat = Mat::new_rows_cols_with_default(height as i32, width as i32, core::CV_8UC3, core::Scalar::all(0.0))?;
    let inv_gamma = 1.0 / gamma;
    let dest_data = final_mat.data_bytes_mut()?;
    
    for (i, &sum) in accumulator.iter().enumerate() {
        let avg_linear = sum / (frame_count as f64);
        let gamma_corrected = avg_linear.powf(inv_gamma) * 255.0;
        dest_data[i] = gamma_corrected.clamp(0.0, 255.0).round() as u8;
    }

    imgcodecs::imwrite(output_path, &final_mat, &core::Vector::default())?;
    println!("Successfully saved averaged image to {}", output_path);
    Ok(())
}

// ==========================================
// 2. MEDIAN (8-PASS BINARY SEARCH)
// ==========================================
fn process_median(input_path: &str, output_path: &str, width: usize, height: usize, total_frames: u64) -> Result<()> {
    let num_elements = width * height * 3;
    let target_count = (total_frames / 2) as u32;
    println!("Target Median Rank: {}", target_count);

    let mut lower = vec![0u8; num_elements];
    let mut upper = vec![255u8; num_elements];

    for pass in 0..8 {
        println!("\n--- Pass {}/8 ---", pass + 1);
        
        let mut pivot = vec![0u8; num_elements];
        for i in 0..num_elements {
            pivot[i] = lower[i] + ((upper[i] - lower[i]) / 2);
        }
        
        let pivot_arc = Arc::new(pivot);
        let pb = ProgressBar::new(total_frames);
        pb.set_style(
            ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} frames ({fps:.1} fps) ETA: {eta}").unwrap().progress_chars("#>-")
        );
        
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(30);
        let producer_pb = pb.clone();
        let input_path_clone = input_path.to_string();

        let producer = thread::spawn(move || -> Result<()> {
            let mut cap = VideoCapture::default()?;
            cap.open_file(&input_path_clone, videoio::CAP_ANY)?;
            let mut frame = Mat::default();
            let mut continuous_frame = Mat::default();

            loop {
                let success = cap.read(&mut frame)?;
                if !success || frame.empty() { break; }

                if !frame.is_continuous() {
                    frame.copy_to(&mut continuous_frame)?;
                } else {
                    continuous_frame = frame.clone();
                }

                let bytes = continuous_frame.data_bytes()?.to_vec();
                if tx.send(bytes).is_err() { break; }
                producer_pb.inc(1);
            }
            producer_pb.finish_with_message("Pass complete.");
            Ok(())
        });

        let count = rx.into_iter()
            .par_bridge()
            .fold(
                || vec![0u32; num_elements],
                |mut local_count: Vec<u32>, frame_data: Vec<u8>| {
                    for (i, (&val, &p)) in frame_data.iter().zip(pivot_arc.iter()).enumerate() {
                        if val <= p { local_count[i] += 1; }
                    }
                    local_count
                }
            )
            .reduce(
                || vec![0u32; num_elements],
                |mut acc1, acc2| {
                    for (a, b) in acc1.iter_mut().zip(acc2) { *a += b; }
                    acc1
                }
            );

        producer.join().expect("Producer thread panicked")?;

        for i in 0..num_elements {
            if count[i] >= target_count {
                upper[i] = pivot_arc[i];
            } else {
                lower[i] = pivot_arc[i] + 1;
            }
        }
    }

    println!("\nGenerating final median image...");
    let mut final_mat = Mat::new_rows_cols_with_default(height as i32, width as i32, core::CV_8UC3, core::Scalar::all(0.0))?;
    let dest_data = final_mat.data_bytes_mut()?;
    dest_data.copy_from_slice(&lower);

    imgcodecs::imwrite(output_path, &final_mat, &core::Vector::default())?;
    println!("Successfully saved median image to {}", output_path);
    Ok(())
}

// ==========================================
// 3. RANDOM (RESERVOIR SAMPLING)
// ==========================================
fn process_random(input_path: &str, output_path: &str, width: usize, height: usize, total_frames: u64) -> Result<()> {
    let num_pixels = width * height;
    let pb = ProgressBar::new(total_frames);
    pb.set_style(
        ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} frames ({fps:.1} fps) ETA: {eta}").unwrap().progress_chars("#>-")
    );

    let (tx, rx) = mpsc::sync_channel::<(u64, Vec<u8>)>(30);
    let producer_pb = pb.clone();
    let input_clone = input_path.to_string();

    thread::spawn(move || -> Result<()> {
        let mut cap = VideoCapture::default()?;
        cap.open_file(&input_clone, videoio::CAP_ANY)?;
        let mut frame_count = 1u64;
        let mut frame = Mat::default();
        let mut continuous_frame = Mat::default();

        loop {
            let success = cap.read(&mut frame)?;
            if !success || frame.empty() { break; }

            if !frame.is_continuous() {
                frame.copy_to(&mut continuous_frame)?;
            } else {
                continuous_frame = frame.clone();
            }

            let bytes = continuous_frame.data_bytes()?.to_vec();
            if tx.send((frame_count, bytes)).is_err() { break; }
            frame_count += 1;
            producer_pb.inc(1);
        }
        producer_pb.finish_with_message("Video reading complete.");
        Ok(())
    });

    println!("Allocating memory for 100 samples per pixel...");
    let mut reservoirs = vec![[[0u8; 3]; 100]; num_pixels];
    let mut frames_processed = 0u64;

    for (frame_idx, frame_data) in rx {
        frames_processed = frame_idx;
        reservoirs.par_iter_mut().zip(frame_data.par_chunks_exact(3)).for_each(|(res_pixel, frame_pixel)| {
            if frame_idx <= 100 {
                res_pixel[(frame_idx - 1) as usize].copy_from_slice(frame_pixel);
            } else {
                let mut rng = rand::thread_rng();
                let probability = 100.0 / (frame_idx as f64);
                if rng.gen_bool(probability) {
                    let replace_idx = rng.gen_range(0..100);
                    res_pixel[replace_idx].copy_from_slice(frame_pixel);
                }
            }
        });
    }

    if frames_processed == 0 { panic!("Error: No frames were read from the video."); }

    println!("\nCalculating the most common color (mode) per channel...");
    let valid_samples = std::cmp::min(frames_processed, 100) as usize;
    let mut final_image = vec![0u8; width * height * 3];

    final_image.par_chunks_exact_mut(3).zip(reservoirs.par_iter()).for_each(|(final_pixel, res_pixel)| {
        for channel in 0..3 {
            let mut counts = [0u8; 256];
            let mut max_count = 0;
            let mut mode_val = 0;
            
            for sample_idx in 0..valid_samples {
                let val = res_pixel[sample_idx][channel];
                counts[val as usize] += 1;
                if counts[val as usize] > max_count {
                    max_count = counts[val as usize];
                    mode_val = val;
                }
            }
            final_pixel[channel] = mode_val;
        }
    });

    let mut final_mat = Mat::new_rows_cols_with_default(height as i32, width as i32, core::CV_8UC3, core::Scalar::all(0.0))?;
    final_mat.data_bytes_mut()?.copy_from_slice(&final_image);
    imgcodecs::imwrite(output_path, &final_mat, &core::Vector::default())?;
    
    println!("Successfully saved mode-averaged image to {}", output_path);
    Ok(())
}