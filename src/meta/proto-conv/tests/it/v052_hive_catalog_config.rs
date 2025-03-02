// Copyright 2021 Datafuse Labs.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use chrono::TimeZone;
use chrono::Utc;
use common_meta_app::schema::CatalogOption;
use common_meta_app::schema::HiveCatalogOption;
use common_meta_app::storage::StorageS3Config;
use minitrace::func_name;

use crate::common;

// These bytes are built when a new version in introduced,
// and are kept for backward compatibility test.
//
// *************************************************************
// * These messages should never be updated,                   *
// * only be added when a new version is added,                *
// * or be removed when an old version is no longer supported. *
// *************************************************************
//
// The message bytes are built from the output of `proto_conv::test_build_pb_buf()`
#[test]
fn test_decode_v52_hive_catalog() -> anyhow::Result<()> {
    let bs = vec![
        18, 124, 18, 122, 10, 15, 49, 50, 55, 46, 48, 46, 48, 46, 49, 58, 49, 48, 48, 48, 48, 18,
        97, 10, 95, 10, 5, 104, 101, 108, 108, 111, 18, 21, 104, 116, 116, 112, 58, 47, 47, 49, 50,
        55, 46, 48, 46, 48, 46, 49, 58, 57, 57, 48, 48, 26, 24, 100, 97, 116, 97, 98, 101, 110,
        100, 95, 104, 97, 115, 95, 115, 117, 112, 101, 114, 95, 112, 111, 119, 101, 114, 34, 24,
        100, 97, 116, 97, 98, 101, 110, 100, 95, 104, 97, 115, 95, 115, 117, 112, 101, 114, 95,
        112, 111, 119, 101, 114, 42, 5, 119, 111, 114, 108, 100, 160, 6, 52, 168, 6, 24, 160, 6,
        52, 168, 6, 24, 162, 1, 23, 50, 48, 49, 52, 45, 49, 49, 45, 50, 56, 32, 49, 50, 58, 48, 48,
        58, 48, 57, 32, 85, 84, 67, 160, 6, 52, 168, 6, 24,
    ];

    let want = || common_meta_app::schema::CatalogMeta {
        catalog_option: CatalogOption::Hive(HiveCatalogOption {
            address: "127.0.0.1:10000".to_string(),
            storage_params: Some(Box::new(common_meta_app::storage::StorageParams::S3(
                StorageS3Config {
                    endpoint_url: "http://127.0.0.1:9900".to_string(),
                    region: "hello".to_string(),
                    bucket: "world".to_string(),
                    access_key_id: "databend_has_super_power".to_string(),
                    secret_access_key: "databend_has_super_power".to_string(),
                    ..Default::default()
                },
            ))),
        }),
        created_on: Utc.with_ymd_and_hms(2014, 11, 28, 12, 0, 9).unwrap(),
    };

    common::test_pb_from_to(func_name!(), want())?;
    common::test_load_old(func_name!(), bs.as_slice(), 52, want())?;

    Ok(())
}
