#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Open,
    Adult,
    Hidden,
}

const ADULT: &[&str] = &[
    "порно",
    "порну",
    "эротическ",
    "хентай",
    "hentai",
    "onlyfans",
    "porn",
];

const SUBSTANCES: &[&str] = &[
    "мефедрон",
    "альфа-пвп",
    "альфапвп",
    "гашиш",
    "героин",
    "кокаин",
    "спайс",
    "экстази",
    "амфетамин",
    "марихуан",
];

const SALE: &[&str] = &[
    "купить",
    "продажа",
    "продаём",
    "продаем",
    "магазин",
    "доставка",
    "закладк",
    "оптом",
];

const DOORWAY: &[&str] = &[
    "перейти на сайт партн",
    "рекламная статья",
    "сгенерировано нейросет",
    "нейросеть сгенерир",
    "заработок на перепродаже трафика",
];

pub fn page_lane(url: &str, title: &str, text: &str) -> Lane {
    let blob = format!("{url} {title} {text}").to_lowercase();
    if is_hidden(&blob) {
        Lane::Hidden
    } else if is_adult(&blob) || host_is_adult(url) {
        Lane::Adult
    } else {
        Lane::Open
    }
}

pub fn query_lane(query: &str) -> Lane {
    let blob = query.to_lowercase();
    if is_hidden(&blob) {
        Lane::Hidden
    } else if is_adult(&blob) {
        Lane::Adult
    } else {
        Lane::Open
    }
}

pub fn is_doorway(text: &str) -> bool {
    let lower = text.to_lowercase();
    DOORWAY.iter().any(|phrase| lower.contains(phrase))
}

fn is_hidden(blob: &str) -> bool {
    if blob.contains("детское порно")
        || blob.contains("детская порнограф")
        || blob.contains("child porn")
        || blob.contains("childporn")
    {
        return true;
    }
    let minor = blob.contains("малолет") || blob.contains("несовершеннолет");
    let sexual = blob.contains("порно") || blob.contains("porn");
    if minor && sexual {
        return true;
    }
    let substance = SUBSTANCES.iter().any(|word| blob.contains(word));
    let sale = SALE.iter().any(|word| blob.contains(word));
    substance && sale
}

fn is_adult(blob: &str) -> bool {
    if ADULT.iter().any(|word| blob.contains(word)) {
        return true;
    }
    has_token(blob, "xxx")
}

fn host_is_adult(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("://porn.") || lower.contains(".porn/") || lower.contains(".xxx/") || lower.contains("://xxx.")
}

fn has_token(text: &str, token: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric()).any(|part| part == token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adult_page_is_separated_and_illegal_sale_is_hidden() {
        assert_eq!(
            page_lane("https://studio.example/v", "Порно видео", "ролик"),
            Lane::Adult
        );
        assert_eq!(
            page_lane("https://farm.example/", "Натуральный мёд", "пасека"),
            Lane::Open
        );
        assert_eq!(
            page_lane("https://shop.example/", "Купить мефедрон", "доставка"),
            Lane::Hidden
        );
        assert_eq!(query_lane("детское порно"), Lane::Hidden);
        assert!(is_doorway("Рекламная статья. Перейти на сайт партнера."));
    }
}
