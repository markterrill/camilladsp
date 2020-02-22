use audiodevice::AudioChunk;
use basicfilters;
use biquad;
use config;
use fftconv;
use mixer;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;

use PrcFmt;
use Res;

pub trait Filter {
    // Filter a Vec
    fn process_waveform(&mut self, waveform: &mut Vec<PrcFmt>) -> Res<()>;
}

pub fn read_coeff_file(filename: &str) -> Res<Vec<PrcFmt>> {
    let mut coefficients = Vec::<PrcFmt>::new();
    let f = File::open(filename)?;
    let file = BufReader::new(&f);
    for line in file.lines() {
        let l = line?;
        coefficients.push(l.trim().parse()?);
    }
    Ok(coefficients)
}

pub struct FilterGroup {
    channel: usize,
    filters: Vec<Box<dyn Filter>>,
}

impl FilterGroup {
    /// Creates a group of filters to process a chunk.
    pub fn from_config(
        channel: usize,
        names: Vec<String>,
        filter_configs: HashMap<String, config::Filter>,
        waveform_length: usize,
        sample_freq: usize,
    ) -> Self {
        let mut filters = Vec::<Box<dyn Filter>>::new();
        for name in names {
            let filter_cfg = filter_configs[&name].clone();
            let filter: Box<dyn Filter> = match filter_cfg {
                config::Filter::Conv { parameters } => {
                    Box::new(fftconv::FFTConv::from_config(waveform_length, parameters))
                }
                config::Filter::Biquad { parameters } => Box::new(biquad::Biquad::new(
                    biquad::BiquadCoefficients::from_config(sample_freq, parameters),
                )),
                config::Filter::Delay { parameters } => {
                    Box::new(basicfilters::Delay::from_config(sample_freq, parameters))
                }
                config::Filter::Gain { parameters } => {
                    Box::new(basicfilters::Gain::from_config(parameters))
                } //_ => panic!("unknown type")
            };
            filters.push(filter);
        }
        FilterGroup {
            channel,
            filters,
        }
    }

    /// Apply all the filters to an AudioChunk.
    fn process_chunk(&mut self, input: &mut AudioChunk) -> Res<()> {
        for filter in &mut self.filters {
            filter.process_waveform(&mut input.waveforms[self.channel])?;
        }
        Ok(())
    }
}

/// A Pipeline is made up of a series of PipelineSteps,
/// each one can be a single Mixer of a group of Filters
pub enum PipelineStep {
    MixerStep(mixer::Mixer),
    FilterStep(FilterGroup),
}

pub struct Pipeline {
    steps: Vec<PipelineStep>,
}

impl Pipeline {
    /// Create a new pipeline from a configuration structure.
    pub fn from_config(conf: config::Configuration) -> Self {
        let mut steps = Vec::<PipelineStep>::new();
        for step in conf.pipeline {
            match step {
                config::PipelineStep::Mixer { name } => {
                    let mixconf = conf.mixers[&name].clone();
                    let mixer = mixer::Mixer::from_config(mixconf);
                    steps.push(PipelineStep::MixerStep(mixer));
                }
                config::PipelineStep::Filter { channel, names } => {
                    let fltgrp = FilterGroup::from_config(
                        channel,
                        names,
                        conf.filters.clone(),
                        conf.devices.buffersize,
                        conf.devices.samplerate,
                    );
                    steps.push(PipelineStep::FilterStep(fltgrp));
                }
            }
        }
        Pipeline { steps }
    }

    /// Process an AudioChunk by calling either a MixerStep or a FilterStep
    pub fn process_chunk(&mut self, mut chunk: AudioChunk) -> AudioChunk {
        for mut step in &mut self.steps {
            match &mut step {
                PipelineStep::MixerStep(mix) => {
                    chunk = mix.process_chunk(&chunk);
                }
                PipelineStep::FilterStep(flt) => {
                    flt.process_chunk(&mut chunk).unwrap();
                }
            }
        }
        chunk
    }
}

/// Validate the filter config, to give a helpful message intead of a panic.
pub fn validate_filter(filter_config: &config::Filter) -> Res<()> {
    match filter_config {
        config::Filter::Conv { parameters } => fftconv::validate_config(&parameters),
        config::Filter::Biquad { .. } => Ok(()),
        config::Filter::Delay { parameters } => {
            if parameters.delay < 0.0 {
                return Err(Box::new(config::ConfigError::new(
                    "Negative delay specified",
                )));
            }
            Ok(())
        }
        config::Filter::Gain { .. } => Ok(()), //_ => panic!("unknown type")
    }
}
