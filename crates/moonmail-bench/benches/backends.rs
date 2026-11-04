// Panics belong to the benchmark harness, not library error handling.
#![allow(clippy::expect_used)]

use {
    criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main},
    moonmail_bench::{documents, mime_fixtures, plain_message},
    moonmail_core::{IndexChange, MimeParser, ParsedMessage, SearchIndex, SearchQuery},
    moonmail_mime::{MailparseParser, StalwartParser},
    moonmail_search::SqliteIndex,
    std::{hint::black_box, num::NonZeroUsize},
};

fn bench_parser<P: MimeParser>(
    criterion: &mut Criterion,
    name: &str,
    parser: &P,
    inputs: &[(&str, &[u8], ParsedMessage)],
) {
    let mut group = criterion.benchmark_group("mime-owned-extraction");
    for (fixture, raw, expected) in inputs {
        assert_eq!(parser.parse(raw).expect("parse fixture"), *expected);
        group.throughput(Throughput::Bytes(raw.len() as u64));
        group.bench_with_input(BenchmarkId::new(name, fixture), raw, |bencher, raw| {
            bencher.iter(|| black_box(parser.parse(black_box(raw)).expect("parse message")));
        });
    }
    group.finish();
}

fn mime(criterion: &mut Criterion) {
    let large = plain_message(1200);
    let stalwart = StalwartParser::default();
    let inputs = mime_fixtures()
        .into_iter()
        .chain([("large-plain", large.as_slice())])
        .map(|(name, raw)| {
            (
                name,
                raw,
                stalwart.parse(raw).expect("reference extraction"),
            )
        })
        .collect::<Vec<_>>();
    bench_parser(criterion, "mail-parser", &stalwart, &inputs);
    bench_parser(criterion, "mailparse", &MailparseParser, &inputs);
}

// G owns any temporary backing files. It outlives I during setup, use and drop.
// Neither the storage guard nor the concrete engine leaks into the workload.
fn bench_index<I: SearchIndex, G>(
    criterion: &mut Criterion,
    name: &str,
    mut create: impl FnMut() -> (I, G),
) {
    let mut group = criterion.benchmark_group("index-batch-commit");
    for count in [100, 1000] {
        let corpus = documents(count);
        let changes = corpus
            .iter()
            .map(|(id, document)| IndexChange::Upsert(*id, document))
            .collect::<Vec<_>>();
        group.throughput(Throughput::Elements(count));
        group.bench_function(BenchmarkId::new(name, count), |bencher| {
            bencher.iter_batched_ref(
                &mut create,
                |(index, _guard)| index.apply(black_box(&changes)).expect("commit batch"),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();

    let corpus = documents(10_000);
    // Keep the tuple intact: fields drop in order, closing the index before
    // removing its backing directory (also important on Windows).
    let mut prepared = create();
    let index = &mut prepared.0;
    for batch in corpus.chunks(500) {
        let changes = batch
            .iter()
            .map(|(id, document)| IndexChange::Upsert(*id, document))
            .collect::<Vec<_>>();
        index.apply(&changes).expect("populate search corpus");
    }
    let mut group = criterion.benchmark_group("search-warm-10k");
    for (case, term, expected) in [
        ("rare", "uniquecheckpoint", 11),
        ("common", "quarterly", 50),
        ("missing", "absenttoken", 0),
    ] {
        let phrases = [term];
        let query = SearchQuery {
            phrases: &phrases,
            limit: NonZeroUsize::new(50).expect("nonzero benchmark limit"),
        };
        assert_eq!(index.search(query).expect("verify search").len(), expected);
        group.bench_function(BenchmarkId::new(name, case), |bencher| {
            bencher.iter(|| black_box(index.search(black_box(query)).expect("search")));
        });
    }
    group.finish();
}

fn indexes(criterion: &mut Criterion) {
    bench_index(criterion, "sqlite-memory", || {
        (SqliteIndex::in_memory().expect("open memory index"), ())
    });
    bench_index(criterion, "sqlite-wal-full", || {
        let directory = tempfile::tempdir().expect("temporary benchmark directory");
        let index = SqliteIndex::open(directory.path().join("search.db")).expect("open disk index");
        (index, directory)
    });
}

criterion_group!(benches, mime, indexes);
criterion_main!(benches);
