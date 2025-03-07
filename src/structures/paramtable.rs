use std::{collections::BTreeMap, iter};

use bytes::{Buf, Bytes};
#[cfg(feature = "pyo3")]
use pyo3::{exceptions::PyKeyError, prelude::*};
use rs3cache_backend::buf::JString;
use serde::Serialize;

use crate::cache::buf::BufExtra;

/// [`LocationConfig`](crate::definitions::location_configs::LocationConfig)s,
/// items and
/// [`NpcConfig`](crate::definitions::npc_configs::NpcConfig)s can have additional mapping of keys to properties.

#[cfg_attr(feature = "pyo3", pyclass(frozen, get_all))]
#[derive(Serialize, Debug, Clone)]
pub struct ParamTable {
    /// Key:Value pairs of additional properties.
    pub params: BTreeMap<u32, Param>,
}

impl ParamTable {
    /// Constructor for [`ParamTable`]
    pub fn deserialize(buffer: &mut Bytes) -> Self {
        let count = buffer.get_u8().into();
        let params = iter::repeat_with(|| Self::sub_deserialize(buffer)).take(count).collect();
        Self { params }
    }

    fn sub_deserialize(buffer: &mut Bytes) -> (u32, Param) {
        let r#type = buffer.get_u8();

        let key = buffer.get_uint(3) as u32;

        let value = match r#type {
            0 => Param::Integer(buffer.get_i32()),
            1 => Param::String(buffer.get_string()),
            other => unimplemented!("Cannot decode unknown type {}", other),
        };
        (key, value)
    }
}

#[cfg(feature = "pyo3")]
#[pymethods]
impl ParamTable {
    fn get(&self, id: u32) -> PyResult<Option<&Param>> {
        Ok(self.params.get(&id))
    }

    fn __getitem__(&self, id: u32) -> PyResult<&Param> {
        self.params.get(&id).ok_or_else(|| PyKeyError::new_err("key not in table"))
    }
}

/// An additional key:property mapping.
#[derive(Serialize, Debug, Eq, PartialEq, Clone)]
#[serde(untagged)]
pub enum Param {
    /// The integer variant.
    Integer(i32),
    /// The string variant.
    String(JString<Bytes>),
}

#[cfg(feature = "pyo3")]
impl IntoPy<PyObject> for Param {
    fn into_py(self, py: Python) -> PyObject {
        match self {
            Param::Integer(val) => val.into_py(py),
            Param::String(val) => val.into_py(py),
        }
    }
}

#[cfg(feature = "pyo3")]
impl IntoPy<PyObject> for &Param {
    fn into_py(self, py: Python) -> PyObject {
        match self {
            Param::Integer(val) => val.into_py(py),
            Param::String(val) => val.as_ref().into_py(py),
        }
    }
}
