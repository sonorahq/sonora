# login
login-restoring = Перевіряємо збережений сеанс…
login-authorizing = Очікуємо авторизацію у браузері…
login-signed-in = Ви увійшли як { $name }
login-failed-title = Не вдалося увійти
login-problem-region = Spotify не відкриває сесію з країни, де ви перебуваєте. Увійдіть з домашньої країни або змініть країну в акаунті Spotify.
login-problem-credentials = Збережена сесія Spotify більше недійсна. Увійдіть знову.
login-problem-network = Sonora не змогла зв'язатися з музичним сервісом. Перевірте інтернет-з'єднання та спробуйте ще раз.
login-problem-cancelled = Ви закрили сторінку браузера, не підтвердивши вхід. Почніть знову.
login-problem-refused = Spotify відхилив вхід. Зачекайте трохи і спробуйте ще раз.
login-problem-premium = Sonora відтворює музику через Spotify Premium, а цей акаунт його не має. Увійдіть в акаунт з Premium, щоб продовжити.
login-sign-in = Увійти через { $provider }
login-connect-cookies = Вставити cookie вручну
login-cookie-open = Відкрити { $provider }
login-cookie-submit = Продовжити
login-cookie-hint = Вставте сюди заголовок запиту Cookie
login-cookie-step-1 = Відкрийте music.youtube.com і переконайтеся, що ви увійшли в акаунт. Найкраще працює вікно анонімного перегляду.
login-cookie-step-2 = Натисніть F12, відкрийте вкладку «Мережа» та перезавантажте сторінку.
login-cookie-step-3 = Виберіть будь-який запит з назвою «browse» або «next».
login-cookie-step-4 = На вкладці «Заголовки» знайдіть Cookie серед заголовків запиту, клацніть правою кнопкою та скопіюйте значення.
login-cookie-step-note = Вставте значення повністю: панелі «Куки» недостатньо, бо значення має містити SAPISID і __Secure-3PAPISID.
login-cookie-header-title = Вставте cookie { $provider }, щоб завершити вхід
login-cookie-apple-step-3 = Виберіть будь-який запит до { $site }.
login-cookie-apple-note = Вставте значення повністю: воно має містити { $cookie }.
login-cookie-named-title = Вставте cookie { $provider }, щоб завершити вхід
login-cookie-named-hint = Вставте сюди значення cookie
login-cookie-named-step-1 = Відкрийте { $site } і переконайтеся, що ви увійшли в обліковий запис.
login-cookie-named-step-2 = Натисніть F12 і відкрийте вкладку Сховище у Firefox або Application у Chrome.
login-cookie-named-step-3 = Розгорніть Cookies, виберіть { $site } і знайдіть cookie з назвою { $cookie }.
login-cookie-named-step-4 = Двічі клацніть його значення та скопіюйте його.
login-cookie-named-note = Достатньо самого значення { $cookie }. Цілий заголовок Cookie з вкладки Мережа також підійде.
login-guest-title = Гостьовий режим
login-guest-use = Відкрити гостьовий режим
login-usage-consent = Допоможіть нам оцінити, скільки людей користується Sonora.
login-device-code = Введіть цей код на { $url }
login-server-title = Підключіться до свого сервера Subsonic
login-server-detail = Введіть адресу будь-якого сервера Subsonic або OpenSubsonic (Navidrome, Airsonic, Gonic, …), а потім увійдіть за допомогою імені користувача та пароля сервера. Сесія залишається на цьому пристрої.
login-server-hint = https://music.example.com
login-username-hint = Ім'я користувача
login-password-hint = Пароль
login-server-submit = Підключитися
login-account-title = Виберіть акаунт
login-account-detail = У цій сесії виконано вхід у кілька акаунтів Google. Виберіть той, який має використовувати Sonora.
login-choose-title = Вхід у { $provider }
login-choose-detail = Виберіть спосіб входу в { $provider }.

# album and playlist pages

# artist page
artist-eyebrow = Виконавець
artist-monthly-listeners = { $count ->
    [one] { $value } слухач на місяць
    [few] { $value } слухачі на місяць
   *[other] { $value } слухачів на місяць
}
artist-popular = Популярне
artist-popular-eyebrow = Знайомство з виконавцем
artist-popular-empty = Поки немає чого слухати в цього виконавця
artist-releases = Релізи
artist-filter-all = Усі
artist-filter-singles = Сингли
artist-filter-eps = EP
artist-appears-on = З'являється в
album-also-like = Вам також може сподобатися
album-tab-artists = Артисти
album-label = ℗ { $label }

# user profile page
user-eyebrow = Профіль
user-followers = { $count ->
    [one] { $value } підписник
    [few] { $value } підписники
   *[other] { $value } підписників
}
user-following = { $count ->
    [one] { $value } підписка
    [few] { $value } підписки
   *[other] { $value } підписок
}
user-playlists = Публічні плейлісти
user-playlists-empty = Поки немає публічних плейлістів

# home page
home-quick-picks = Швидкий вибір
home-quick-picks-empty = Додайте кілька треків, і вони з'являться тут
home-recently-added = Нещодавно додане
home-recently-played = Нещодавно прослухане
home-favorite-albums = Улюблені альбоми
home-artists = Виконавці
home-collection-albums = Альбоми з вашої колекції

# search page
search-placeholder = Що хочете послухати?
search-browse = Огляд
search-submit = Знайти
search-recent = Нещодавні запити
search-forget = Видалити з історії
genre-empty = Тут поки що порожньо
search-best-match = Найкращий збіг
search-no-matches = Нічого не знайдено
search-results = Результати
search-artists = Виконавці
search-albums-playlists = Альбоми та плейлисти
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } трек у медіатеці
        [few] { $count } треки у медіатеці
       *[other] { $count } треків у медіатеці
    }

# song page
song-eyebrow = Трек
song-view-album = Відкрити альбом
song-about = Про трек
song-album = Альбом
song-released = Дата виходу
song-streams = Прослуховування
song-position = Позиція
song-label = Лейбл
song-popularity = Популярність
song-disc-track = Диск { $disc }, трек { $track }
song-track = Трек { $track }
song-credits = Автори
song-performed-by = Виконання
song-details = Жанри та деталі
song-genres = Жанри
song-language = Мова
song-content = Контент
song-explicit = Ненормативна лексика
song-clean = Без обмежень
artist-about = Про виконавця
artist-about-fallback = Послухайте популярні треки та релізи виконавця.
artist-about-open = Перейти до виконавця
song-copyright = © { $notice }

# tags
tags-edit-title = Змінення тегів
tags-sheet-song = Трек
tags-sheet-album = Альбом
tags-sheet-details = Деталі
tags-title = Назва
tags-artist = Виконавець
tags-artists = Виконавці
tags-add-artist = Додати виконавця
tags-remove-artist = Вилучити цього виконавця
tags-track = Номер треку
tags-track-total = Треків у релізі
tags-disc = Номер диска
tags-disc-total = Дисків у релізі
tags-album = Альбом
tags-album-artist = Виконавець альбому
tags-year = Рік
tags-genre = Жанр
tags-composer = Композитор
tags-publisher = Видавець
tags-isrc = ISRC
tags-comment = Коментар
