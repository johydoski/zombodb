use crate::elasticsearch::Elasticsearch;
use crate::gucs::{ZDB_DEFAULT_ELASTICSEARCH_URL, ZDB_DEFAULT_REPLICAS};
use lazy_static::*;
use memoffset::*;
use pgx::*;
use std::ffi::{CStr, CString};

const DEFAULT_BATCH_SIZE: i32 = 8 * 1024 * 1024;
const DEFAULT_COMPRESSION_LEVEL: i32 = 1;
const DEFAULT_SHARDS: i32 = 5;
const DEFAULT_OPTIMIZE_AFTER: i32 = 0;
const DEFAULT_URL: &str = "default";
const DEFAULT_TYPE_NAME: &str = "doc";
const DEFAULT_REFRESH_INTERVAL: &str = "-1";
const DEFAULT_TRANSLOG_DURABILITY: &str = "request";

lazy_static! {
    static ref DEFAULT_BULK_CONCURRENCY: i32 = num_cpus::get() as i32;
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub enum RefreshInterval {
    Immediate,
    ImmediateAsync,
    Background(String),
}

impl RefreshInterval {
    pub fn as_str(&self) -> &str {
        match self {
            RefreshInterval::Immediate => "-1",
            RefreshInterval::ImmediateAsync => "-1",
            RefreshInterval::Background(s) => s.as_str(),
        }
    }
}

#[repr(C)]
pub struct ZDBIndexOptions {
    /* varlena header (do not touch directly!) */
    #[allow(dead_code)]
    vl_len_: i32,

    url_offset: i32,
    type_name_offset: i32,
    refresh_interval_offset: i32,
    alias_offset: i32,
    uuid_offset: i32,
    translog_durability_offset: i32,

    optimize_after: i32,
    compression_level: i32,
    shards: i32,
    replicas: i32,
    bulk_concurrency: i32,
    batch_size: i32,
    llapi: bool,
}

#[allow(dead_code)]
impl ZDBIndexOptions {
    pub fn from(relation: &PgRelation) -> PgBox<ZDBIndexOptions> {
        if relation.rd_index.is_null() {
            panic!("relation doesn't represent an index")
        } else if relation.rd_options.is_null() {
            // use defaults
            let mut ops = PgBox::<ZDBIndexOptions>::alloc0();
            ops.compression_level = DEFAULT_COMPRESSION_LEVEL;
            ops.shards = DEFAULT_SHARDS;
            ops.replicas = ZDB_DEFAULT_REPLICAS.get();
            ops.bulk_concurrency = *DEFAULT_BULK_CONCURRENCY;
            ops.batch_size = DEFAULT_BATCH_SIZE;
            ops.optimize_after = DEFAULT_OPTIMIZE_AFTER;
            ops
        } else {
            PgBox::from_pg(relation.rd_options as *mut ZDBIndexOptions)
        }
    }

    pub fn optimize_after(&self) -> i32 {
        self.optimize_after
    }

    pub fn compression_level(&self) -> i32 {
        self.compression_level
    }

    pub fn shards(&self) -> i32 {
        self.shards
    }

    pub fn replicas(&self) -> i32 {
        self.replicas
    }

    pub fn bulk_concurrency(&self) -> i32 {
        self.bulk_concurrency
    }

    pub fn batch_size(&self) -> i32 {
        self.batch_size
    }

    pub fn llapi(&self) -> bool {
        self.llapi
    }

    pub fn url(&self) -> String {
        let url = self.get_str(self.url_offset, || DEFAULT_URL.to_owned());

        if url == DEFAULT_URL {
            // the url option on the index could also be the string 'default', so
            // in either case above, lets use the setting from postgresql.conf
            if ZDB_DEFAULT_ELASTICSEARCH_URL.get().is_some() {
                ZDB_DEFAULT_ELASTICSEARCH_URL.get().unwrap()
            } else {
                // the user hasn't provided one
                panic!("Must set zdb.default_elasticsearch_url");
            }
        } else {
            // the index itself has a valid url
            url
        }
    }

    pub fn type_name(&self) -> String {
        self.get_str(self.type_name_offset, || DEFAULT_TYPE_NAME.to_owned())
    }

    pub fn refresh_interval(&self) -> RefreshInterval {
        match self
            .get_str(self.refresh_interval_offset, || {
                DEFAULT_REFRESH_INTERVAL.to_owned()
            })
            .as_str()
        {
            "-1" | "immediate" => RefreshInterval::Immediate,
            "async" => RefreshInterval::ImmediateAsync,
            other => RefreshInterval::Background(other.to_owned()),
        }
    }

    pub fn alias(&self, heaprel: &PgRelation, indexrel: &PgRelation) -> String {
        self.get_str(self.alias_offset, || {
            format!(
                "{}.{}.{}.{}-{}",
                unsafe {
                    std::ffi::CStr::from_ptr(pg_sys::get_database_name(pg_sys::MyDatabaseId))
                }
                .to_str()
                .unwrap(),
                unsafe {
                    std::ffi::CStr::from_ptr(pg_sys::get_namespace_name(indexrel.namespace_oid()))
                }
                .to_str()
                .unwrap(),
                heaprel.name(),
                indexrel.name(),
                indexrel.oid()
            )
        })
    }

    pub fn uuid(&self, heaprel: &PgRelation, indexrel: &PgRelation) -> String {
        self.get_str(self.uuid_offset, || {
            format!(
                "{}.{}.{}.{}",
                unsafe { pg_sys::MyDatabaseId },
                indexrel.namespace_oid(),
                heaprel.oid(),
                indexrel.oid(),
            )
        })
    }

    pub fn index_name(&self, heaprel: &PgRelation, indexrel: &PgRelation) -> String {
        self.uuid(heaprel, indexrel)
    }

    pub fn translog_durability(&self) -> String {
        self.get_str(self.translog_durability_offset, || {
            DEFAULT_TRANSLOG_DURABILITY.to_owned()
        })
    }

    fn get_str<F: FnOnce() -> String>(&self, offset: i32, default: F) -> String {
        if offset == 0 {
            default()
        } else {
            let opts = self as *const _ as void_ptr as usize;
            let value =
                unsafe { CStr::from_ptr((opts + offset as usize) as *const std::os::raw::c_char) };

            value.to_str().unwrap().to_owned()
        }
    }
}

#[pg_extern(immutable, parallel_safe)]
fn index_name(index_relation: PgRelation) -> String {
    let heap_relation = index_relation
        .heap_relation()
        .expect("relation is not an index relation");
    ZDBIndexOptions::from(&index_relation).index_name(&heap_relation, &index_relation)
}

#[pg_extern(immutable, parallel_safe)]
fn index_url(index_relation: PgRelation) -> String {
    ZDBIndexOptions::from(&index_relation).url()
}

#[pg_extern(immutable, parallel_safe)]
fn index_type_name(index_relation: PgRelation) -> String {
    ZDBIndexOptions::from(&index_relation).type_name()
}

#[pg_extern(immutable, parallel_safe)]
fn index_mapping(index_relation: PgRelation) -> JsonB {
    JsonB(
        Elasticsearch::new(&index_relation)
            .get_mapping()
            .execute()
            .expect("failed to get index mapping"),
    )
}

static mut RELOPT_KIND_ZDB: pg_sys::relopt_kind = 0;

extern "C" fn validate_url(url: *const std::os::raw::c_char) {
    let url = unsafe { CStr::from_ptr(url) }
        .to_str()
        .expect("failed to convert url to utf8");

    if url == "default" {
        // "default" is a fine value
        return;
    }

    if !url.ends_with('/') {
        panic!("url must end with a forward slash");
    }

    if let Err(e) = url::Url::parse(url) {
        panic!(e.to_string())
    }
}

extern "C" fn validate_translog_durability(value: *const std::os::raw::c_char) {
    if value.is_null() {
        // null is fine -- we'll just use our default
        return;
    }

    let value = unsafe { CStr::from_ptr(value) }
        .to_str()
        .expect("failed to convert translog_durability to utf8");
    if value != "request" && value != "async" {
        panic!(
            "invalid translog_durability setting.  Must be one of 'request' or 'async': {}",
            value
        )
    }
}

#[allow(clippy::unneeded_field_pattern)] // b/c of offset_of!()
#[pg_guard]
pub unsafe extern "C" fn amoptions(
    reloptions: pg_sys::Datum,
    validate: bool,
) -> *mut pg_sys::bytea {
    // TODO:  how to make this const?  we can't use offset_of!() macro in const definitions, apparently
    let tab: [pg_sys::relopt_parse_elt; 13] = [
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"url\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_STRING,
            offset: offset_of!(ZDBIndexOptions, url_offset) as i32,
        },
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"type_name\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_STRING,
            offset: offset_of!(ZDBIndexOptions, type_name_offset) as i32,
        },
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"refresh_interval\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_STRING,
            offset: offset_of!(ZDBIndexOptions, refresh_interval_offset) as i32,
        },
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"shards\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_INT,
            offset: offset_of!(ZDBIndexOptions, shards) as i32,
        },
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"replicas\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_INT,
            offset: offset_of!(ZDBIndexOptions, replicas) as i32,
        },
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"bulk_concurrency\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_INT,
            offset: offset_of!(ZDBIndexOptions, bulk_concurrency) as i32,
        },
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"batch_size\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_INT,
            offset: offset_of!(ZDBIndexOptions, batch_size) as i32,
        },
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"compression_level\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_INT,
            offset: offset_of!(ZDBIndexOptions, compression_level) as i32,
        },
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"alias\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_STRING,
            offset: offset_of!(ZDBIndexOptions, alias_offset) as i32,
        },
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"optimize_after\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_INT,
            offset: offset_of!(ZDBIndexOptions, optimize_after) as i32,
        },
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"llapi\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_BOOL,
            offset: offset_of!(ZDBIndexOptions, llapi) as i32,
        },
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"uuid\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_STRING,
            offset: offset_of!(ZDBIndexOptions, uuid_offset) as i32,
        },
        pg_sys::relopt_parse_elt {
            optname: CStr::from_bytes_with_nul_unchecked(b"translog_durability\0").as_ptr(),
            opttype: pg_sys::relopt_type_RELOPT_TYPE_STRING,
            offset: offset_of!(ZDBIndexOptions, translog_durability_offset) as i32,
        },
    ];

    let mut noptions = 0;
    let options = pg_sys::parseRelOptions(reloptions, validate, RELOPT_KIND_ZDB, &mut noptions);
    if noptions == 0 {
        return std::ptr::null_mut();
    }

    for relopt in std::slice::from_raw_parts_mut(options, noptions as usize) {
        relopt.gen.as_mut().unwrap().lockmode = pg_sys::AccessShareLock as pg_sys::LOCKMODE;
    }

    let rdopts =
        pg_sys::allocateReloptStruct(std::mem::size_of::<ZDBIndexOptions>(), options, noptions);
    pg_sys::fillRelOptions(
        rdopts,
        std::mem::size_of::<ZDBIndexOptions>(),
        options,
        noptions,
        validate,
        tab.as_ptr(),
        tab.len() as i32,
    );
    pg_sys::pfree(options as void_mut_ptr);

    rdopts as *mut pg_sys::bytea
}

pub unsafe fn init() {
    RELOPT_KIND_ZDB = pg_sys::add_reloption_kind();
    pg_sys::add_string_reloption(
        RELOPT_KIND_ZDB,
        CStr::from_bytes_with_nul_unchecked(b"url\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(b"Server URL and port\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(b"default\0").as_ptr(),
        Some(validate_url),
    );
    pg_sys::add_string_reloption(
        RELOPT_KIND_ZDB,
        CStr::from_bytes_with_nul_unchecked(b"type_name\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(
            b"What Elasticsearch index type name should ZDB use?  Default is 'doc'\0",
        )
        .as_ptr(),
        CStr::from_bytes_with_nul_unchecked(b"doc\0").as_ptr(),
        None,
    );
    let default_refresh_interval = CString::new(DEFAULT_REFRESH_INTERVAL).unwrap();
    pg_sys::add_string_reloption(RELOPT_KIND_ZDB, CStr::from_bytes_with_nul_unchecked(b"refresh_interval\0").as_ptr(),
                                 CStr::from_bytes_with_nul_unchecked(b"Frequency in which Elasticsearch indexes are refreshed.  Related to ES' index.refresh_interval setting\0").as_ptr(),
                                 default_refresh_interval.as_ptr(), None);
    pg_sys::add_int_reloption(
        RELOPT_KIND_ZDB,
        CStr::from_bytes_with_nul_unchecked(b"shards\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(b"The number of shards for the index\0").as_ptr(),
        DEFAULT_SHARDS,
        1,
        32768,
    );
    pg_sys::add_int_reloption(
        RELOPT_KIND_ZDB,
        CStr::from_bytes_with_nul_unchecked(b"replicas\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(b"The number of replicas for the index\0").as_ptr(),
        ZDB_DEFAULT_REPLICAS.get(),
        0,
        32768,
    );
    pg_sys::add_int_reloption(
        RELOPT_KIND_ZDB,
        CStr::from_bytes_with_nul_unchecked(b"bulk_concurrency\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(
            b"The maximum number of concurrent _bulk API requests\0",
        )
        .as_ptr(),
        *DEFAULT_BULK_CONCURRENCY,
        1,
        num_cpus::get() as i32,
    );
    pg_sys::add_int_reloption(
        RELOPT_KIND_ZDB,
        CStr::from_bytes_with_nul_unchecked(b"batch_size\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(b"The size in bytes of batch calls to the _bulk API\0")
            .as_ptr(),
        DEFAULT_BATCH_SIZE,
        1,
        (std::i32::MAX / 2) - 1,
    );
    pg_sys::add_int_reloption(
        RELOPT_KIND_ZDB,
        CStr::from_bytes_with_nul_unchecked(b"compression_level\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(
            b"0-9 value to indicate the level of HTTP compression\0",
        )
        .as_ptr(),
        DEFAULT_COMPRESSION_LEVEL,
        0,
        9,
    );
    pg_sys::add_string_reloption(
        RELOPT_KIND_ZDB,
        CStr::from_bytes_with_nul_unchecked(b"alias\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(
            b"The Elasticsearch Alias to which this index should belong\0",
        )
        .as_ptr(),
        std::ptr::null(),
        None,
    );
    pg_sys::add_string_reloption(
        RELOPT_KIND_ZDB,
        CStr::from_bytes_with_nul_unchecked(b"uuid\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(b"The Elasticsearch index name, as a UUID\0").as_ptr(),
        std::ptr::null(),
        None,
    );
    pg_sys::add_string_reloption(
        RELOPT_KIND_ZDB,
        CStr::from_bytes_with_nul_unchecked(b"translog_durability\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(
            b"Elasticsearch index.translog.durability setting.  Defaults to 'request'",
        )
        .as_ptr(),
        CStr::from_bytes_with_nul_unchecked(b"request\0").as_ptr(),
        Some(validate_translog_durability),
    );
    pg_sys::add_int_reloption(
        RELOPT_KIND_ZDB,
        CStr::from_bytes_with_nul_unchecked(b"optimize_after\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(
            b"After how many deleted docs should ZDB _optimize the ES index during VACUUM?\0",
        )
        .as_ptr(),
        DEFAULT_OPTIMIZE_AFTER,
        0,
        std::i32::MAX,
    );
    pg_sys::add_bool_reloption(
        RELOPT_KIND_ZDB,
        CStr::from_bytes_with_nul_unchecked(b"llapi\0").as_ptr(),
        CStr::from_bytes_with_nul_unchecked(
            b"Will this index be used by ZomboDB's low-level API?\0",
        )
        .as_ptr(),
        false,
    );
}

#[cfg(any(test, feature = "pg_test"))]
mod tests {
    use crate::access_method::options::{
        validate_translog_durability, validate_url, RefreshInterval, ZDBIndexOptions,
        DEFAULT_BATCH_SIZE, DEFAULT_BULK_CONCURRENCY, DEFAULT_COMPRESSION_LEVEL,
        DEFAULT_OPTIMIZE_AFTER, DEFAULT_SHARDS, DEFAULT_TYPE_NAME,
    };
    use crate::gucs::ZDB_DEFAULT_REPLICAS;
    use pgx::*;
    use std::ffi::CString;

    #[pg_test]
    fn test_validate_url() {
        validate_url(CString::new("http://localhost:9200/").unwrap().as_ptr());
    }

    #[pg_test]
    fn test_validate_default_url() {
        validate_url(CString::new("default").unwrap().as_ptr());
    }

    #[pg_test(error = "url must end with a forward slash")]
    fn test_validate_invalid_url() {
        validate_url(CString::new("http://localhost:9200").unwrap().as_ptr());
    }

    #[pg_test(
        error = "invalid translog_durability setting.  Must be one of 'request' or 'async': foo"
    )]
    fn test_validate_invalid_translog_durability() {
        validate_translog_durability(CString::new("foo").unwrap().as_ptr());
    }

    #[test]
    fn test_valid_translog_durability_request() {
        validate_translog_durability(CString::new("request").unwrap().as_ptr());
    }

    #[test]
    fn test_valid_translog_durability_async() {
        validate_translog_durability(CString::new("async").unwrap().as_ptr());
    }

    #[pg_test]
    #[initialize(es = true)]
    unsafe fn test_index_options() {
        let uuid = rand::random::<u64>();
        Spi::run(&format!(
            "CREATE TABLE test();  
        CREATE INDEX idxtest 
                  ON test 
               USING zombodb ((test.*)) 
                WITH (url='http://localhost:19200/', 
                      type_name='test_type_name', 
                      alias='test_alias', 
                      uuid='{}', 
                      refresh_interval='5s',
                      translog_durability='async');",
            uuid
        ));

        let heap_oid = Spi::get_one::<pg_sys::Oid>("SELECT 'test'::regclass::oid")
            .expect("failed to get SPI result");
        let index_oid = Spi::get_one::<pg_sys::Oid>("SELECT 'idxtest'::regclass::oid")
            .expect("failed to get SPI result");
        let heaprel = PgRelation::from_pg(pg_sys::RelationIdGetRelation(heap_oid));
        let indexrel = PgRelation::from_pg(pg_sys::RelationIdGetRelation(index_oid));
        let options = ZDBIndexOptions::from(&indexrel);
        assert_eq!(&options.url(), "http://localhost:19200/");
        assert_eq!(&options.type_name(), "test_type_name");
        assert_eq!(&options.alias(&heaprel, &indexrel), "test_alias");
        assert_eq!(&options.uuid(&heaprel, &indexrel), &uuid.to_string());
        assert_eq!(
            options.refresh_interval(),
            RefreshInterval::Background("5s".to_owned())
        );
        assert_eq!(options.compression_level(), 1);
        assert_eq!(options.shards(), 5);
        assert_eq!(options.replicas(), 0);
        assert_eq!(options.bulk_concurrency(), num_cpus::get() as i32);
        assert_eq!(options.batch_size(), 8 * 1024 * 1024);
        assert_eq!(options.optimize_after(), DEFAULT_OPTIMIZE_AFTER);
        assert_eq!(options.llapi(), false);
        assert_eq!(options.translog_durability(), "async")
    }

    #[pg_test]
    #[initialize(es = true)]
    unsafe fn test_index_options_defaults() {
        Spi::run(
            "CREATE TABLE test();  
        CREATE INDEX idxtest 
                  ON test 
               USING zombodb ((test.*)) WITH (url='http://localhost:19200/');",
        );

        let heap_oid = Spi::get_one::<pg_sys::Oid>("SELECT 'test'::regclass::oid")
            .expect("failed to get SPI result");
        let index_oid = Spi::get_one::<pg_sys::Oid>("SELECT 'idxtest'::regclass::oid")
            .expect("failed to get SPI result");
        let heaprel = PgRelation::from_pg(pg_sys::RelationIdGetRelation(heap_oid));
        let indexrel = PgRelation::from_pg(pg_sys::RelationIdGetRelation(index_oid));
        let options = ZDBIndexOptions::from(&indexrel);
        assert_eq!(&options.type_name(), DEFAULT_TYPE_NAME);
        assert_eq!(
            &options.alias(&heaprel, &indexrel),
            &format!("pgx_tests.public.test.idxtest-{}", indexrel.oid())
        );
        assert_eq!(
            &options.uuid(&heaprel, &indexrel),
            &format!(
                "{}.{}.{}.{}",
                pg_sys::MyDatabaseId,
                indexrel.namespace_oid(),
                heaprel.oid(),
                indexrel.oid()
            )
        );
        assert_eq!(options.refresh_interval(), RefreshInterval::Immediate);
        assert_eq!(options.compression_level(), DEFAULT_COMPRESSION_LEVEL);
        assert_eq!(options.shards(), DEFAULT_SHARDS);
        assert_eq!(options.replicas(), ZDB_DEFAULT_REPLICAS.get());
        assert_eq!(options.bulk_concurrency(), *DEFAULT_BULK_CONCURRENCY);
        assert_eq!(options.batch_size(), DEFAULT_BATCH_SIZE);
        assert_eq!(options.optimize_after(), DEFAULT_OPTIMIZE_AFTER);
        assert_eq!(options.llapi(), false);
        assert_eq!(options.translog_durability(), "request")
    }

    #[pg_test]
    #[initialize(es = true)]
    unsafe fn test_index_name() {
        Spi::run(
            "CREATE TABLE test();  
        CREATE INDEX idxtest 
                  ON test 
               USING zombodb ((test.*)) WITH (url='http://localhost:19200/');",
        );

        let index_relation = PgRelation::open_with_name("idxtest").expect("no such relation");
        let heap_relation = index_relation.heap_relation().unwrap();
        let options = ZDBIndexOptions::from(&index_relation);

        assert_eq!(
            options.index_name(&heap_relation, &index_relation),
            options.uuid(&heap_relation, &index_relation)
        );
    }

    #[pg_test]
    #[initialize(es = true)]
    unsafe fn test_index_url() {
        Spi::run(
            "CREATE TABLE test();  
        CREATE INDEX idxtest 
                  ON test 
               USING zombodb ((test.*)) WITH (url='http://localhost:19200/');",
        );

        let index_relation = PgRelation::open_with_name("idxtest").expect("no such relation");
        let options = ZDBIndexOptions::from(&index_relation);

        assert_eq!(options.url(), "http://localhost:19200/");
    }

    #[pg_test]
    #[initialize(es = true)]
    unsafe fn test_index_type_name() {
        Spi::run(
            "CREATE TABLE test();  
        CREATE INDEX idxtest 
                  ON test 
               USING zombodb ((test.*)) WITH (url='http://localhost:19200/');",
        );

        let index_relation = PgRelation::open_with_name("idxtest").expect("no such relation");
        let options = ZDBIndexOptions::from(&index_relation);

        assert_eq!(options.type_name(), "doc");
    }
}
