//! Generic parsing function implementation
//! Provide way to load various trace from bincode format and extract polars DataFrame format

use super::*;
use thiserror::Error;

// Define some common properties for trace files
const BINCODE_STREAM_DELIMITER: u8 = 0xA;
const BINCODE_BUFFER_SIZE: usize = 0x1000;
const BINCODE_MAX_RETRY: usize = 10;

/// Parse T from bincode trace file.
/// return a HashMap of T (sorted by port_name)
pub fn read_trace<T: for<'a> serde::Deserialize<'a> + std::fmt::Debug>(
    path: &str,
) -> Result<HashMap<String, Vec<T>>, anyhow::Error> {
    // Read ra2m trace and convert it into parseable structure
    let mut bc_reader = BufReader::new(File::open(path).unwrap());
    let mut map: HashMap<String, Vec<T>> = HashMap::new();
    let mut buf = Vec::with_capacity(BINCODE_BUFFER_SIZE);

    let mut bincode_retry = 0;
    let mut bincode_success = 0;
    loop {
        if 0 == bincode_retry {
            buf.clear();
        }

        match bc_reader.read_until(BINCODE_STREAM_DELIMITER, &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                match bincode::deserialize::<(String, T)>(buf.as_slice()) {
                    Ok((port, val)) => {
                        tracing::debug!("{port} => {val:?}");
                        bincode_retry = 0;
                        bincode_success += 1;
                        map.entry(port).or_insert(Vec::new()).push(val);
                    }
                    Err(err) => {
                        // Error could came from present of 0xA in bincode stream. In this case
                        // append the next chunk to buf and retry decoding.
                        // For sanity purpose, limit the number of retry to BINCODE_MAX_RETRY
                        // Stop retry when buffer overflow preallocated capacity
                        // Last word could be incomplete (since ra2m simulation is usually stopped with Ctrl-C),
                        // Thus in case of error with already parsed stuff, return success
                        bincode_retry += 1;
                        if bincode_retry > BINCODE_MAX_RETRY {
                            if bincode_success > 0 {
                                break;
                            } else {
                                return Err(err.into());
                            }
                        }
                    }
                }
            }
            Err(err) => return Err(err.into()),
        }
    }
    Ok(map)
}

/// Convert a list of column name with associated generic type in a Polars DataFrame
pub fn to_dataframe(data: HashMap<&str, Vec<Traceable>>) -> Result<DataFrame, ParserError> {
    let column: Result<Vec<Column>, ParserError> = data
        .into_iter()
        .map(|(name, traceables)| match traceables.first() {
            Some(Traceable::Text(_)) => {
                let values: Result<Vec<String>, _> = traceables
                    .into_iter()
                    .map(|t| match t {
                        Traceable::Text(s) => Ok(s),
                        _ => Err(ParserError::MixedTypes(name.to_string())),
                    })
                    .collect();
                Ok(Column::from(Series::new(name.into(), values?)))
            }
            Some(Traceable::Scalar(_)) => {
                let values: Result<Vec<u64>, _> = traceables
                    .into_iter()
                    .map(|t| match t {
                        Traceable::Scalar(v) => Ok(v),
                        _ => Err(ParserError::MixedTypes(name.to_string())),
                    })
                    .collect();
                Ok(Column::from(Series::new(name.into(), values?)))
            }
            Some(Traceable::Array(_)) => {
                let values: Result<Vec<Series>, _> = traceables
                    .into_iter()
                    .map(|t| match t {
                        Traceable::Array(data) => Ok(Series::new("".into(), data)),
                        _ => Err(ParserError::MixedTypes(name.to_string())),
                    })
                    .collect();
                Ok(Column::from(Series::new(name.into(), values?.as_slice())))
            }
            None => Ok(Column::from(Series::new_empty(
                name.into(),
                &DataType::Null,
            ))),
        })
        .collect();

    DataFrame::new(column?).map_err(Into::into)
}

/// Error gathering between Polars and internal data-type mismatch
#[derive(Debug, Error)]
pub enum ParserError {
    #[error("Multiple type mixed in column {0}")]
    MixedTypes(String),
    #[error("Polars internal error {0}")]
    Polars(PolarsError),
}

impl From<PolarsError> for ParserError {
    fn from(err: PolarsError) -> Self {
        ParserError::Polars(err)
    }
}
