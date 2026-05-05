use polars::prelude::*;
use ra2m_sim::prelude::*;
use std::collections::HashMap;
use std::{
    fs::File,
    io::{BufRead, BufReader},
};

#[derive(Debug, Clone, strum_macros::EnumString)]
pub enum TraceKind {
    MemBus,
    DmaBus,
}

mod parser;
pub use parser::{read_trace, to_dataframe, ParserError};
