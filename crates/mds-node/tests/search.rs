use std::path::PathBuf;
use std::time::Duration;

use mds_crawl::CrawlPolicy;
use mds_node::{index_fixture_dir, Engine};

#[tokio::test]
async fn finds_exact_token_and_semantic_page() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let db = std::env::temp_dir().join(format!("mds-search-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let mut engine = Engine::open(&db, &root.join("data/models")).expect("модель");
    let stored = index_fixture_dir(
        &mut engine,
        &root.join("fixtures"),
        &CrawlPolicy {
            delay: Duration::from_millis(0),
            max_bytes: 200_000,
            max_pages: 10,
            follow_external: false,
        },
    )
    .await
    .expect("обход");
    assert_eq!(stored, 3, "закрытая страница не должна попасть в индекс, сохранено {stored}");

    let exact = engine.search("MDSBOLT42").expect("точный запрос");
    assert!(
        exact.iter().any(|hit| hit.url.contains("bolt") && hit.via_words),
        "точный токен не найден: {exact:?}"
    );

    let semantic = engine.search("как лечить собаку").expect("смысловой запрос");
    let vet = semantic
        .iter()
        .find(|hit| hit.url.contains("vet"))
        .unwrap_or_else(|| panic!("ветеринарная страница не найдена: {semantic:?}"));
    assert!(vet.via_meaning, "страница найдена не вектором: {vet:?}");
    assert!(!vet.via_words, "сработали слова, а не смысл: {vet:?}");
    assert!(
        semantic[0].url.contains("vet"),
        "смысловая страница должна быть первой: {semantic:?}"
    );

    let hidden = engine.search("SECRETTOKEN99").expect("секрет");
    assert!(
        hidden.iter().all(|hit| !hit.url.contains("secret")),
        "закрытая страница попала в индекс: {hidden:?}"
    );

    let unrelated = engine.search("уксус").expect("постороннее слово");
    assert!(
        unrelated.is_empty(),
        "посторонний запрос 'уксус' не должен возвращать ложные совпадения по смыслу: {unrelated:?}"
    );
    let honey = engine.search("купить мёд").expect("купить мёд");
    assert!(
        honey.is_empty(),
        "запрос 'купить мёд' не должен вытягивать чужие страницы: {honey:?}"
    );
    let yo_strict = engine.search("крепеж MDSBOLT42").expect("запрос с е вместо ё");
    assert!(
        yo_strict.iter().all(|hit| !hit.via_words),
        "буквы 'е' и 'ё' должны строго различаться в словарном индексе: {yo_strict:?}"
    );
    let _ = std::fs::remove_file(&db);
}
