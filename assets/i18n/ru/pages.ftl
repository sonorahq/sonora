members-title = Участники
members-empty = Начните сессию или присоединитесь к ней, чтобы увидеть, кто слушает вместе с вами

# login
login-restoring = Проверяем сохранённую сессию…
login-authorizing = Ожидаем авторизацию в браузере…
login-signed-in = Вы вошли как { $name }
login-failed-title = Не удалось войти
login-problem-region = Spotify не открывает сессию из страны, где вы находитесь. Войдите из домашней страны или смените страну в аккаунте Spotify.
login-problem-credentials = Сохранённая сессия Spotify больше недействительна. Войдите заново.
login-problem-network = Sonora не смогла связаться с музыкальным сервисом. Проверьте интернет-соединение и попробуйте снова.
login-problem-cancelled = Вы закрыли страницу браузера, не подтвердив вход. Начните заново.
login-problem-refused = Spotify отклонил вход. Подождите немного и попробуйте снова.
login-problem-premium = Sonora играет музыку через Spotify Premium, а у этого аккаунта его нет. Войдите в аккаунт с Premium, чтобы продолжить.
login-sign-in = Войти через { $provider }
login-connect-cookies = Вставить cookie вручную
login-cookie-open = Открыть { $provider }
login-cookie-submit = Продолжить
login-cookie-hint = Вставьте сюда заголовок запроса Cookie
login-cookie-step-1 = Откройте music.youtube.com и убедитесь, что вы вошли в аккаунт. Лучше всего работает окно в режиме инкогнито.
login-cookie-step-2 = Нажмите F12, откройте вкладку «Сеть» и перезагрузите страницу.
login-cookie-step-3 = Выберите любой запрос с именем «browse» или «next».
login-cookie-step-4 = На вкладке «Заголовки» найдите Cookie в заголовках запроса, щёлкните правой кнопкой и скопируйте значение.
login-cookie-step-note = Вставьте значение целиком: панели «Куки» недостаточно, потому что значение должно содержать SAPISID и __Secure-3PAPISID.
login-cookie-header-title = Вставьте cookie { $provider }, чтобы завершить вход
login-cookie-apple-step-3 = Выберите любой запрос к { $site }.
login-cookie-apple-note = Вставьте значение целиком: оно должно содержать { $cookie }.
login-cookie-named-title = Вставьте cookie { $provider }, чтобы завершить вход
login-cookie-named-hint = Вставьте сюда значение cookie
login-cookie-named-step-1 = Откройте { $site } и убедитесь, что вы вошли в аккаунт.
login-cookie-named-step-2 = Нажмите F12 и откройте вкладку Хранилище в Firefox или Application в Chrome.
login-cookie-named-step-3 = Раскройте Cookies, выберите { $site } и найдите cookie с именем { $cookie }.
login-cookie-named-step-4 = Дважды щёлкните по его значению и скопируйте его.
login-cookie-named-note = Достаточно одного значения { $cookie }. Целый заголовок Cookie из вкладки Сеть тоже подойдёт.
login-guest-title = Гостевой режим
login-guest-use = Открыть гостевой режим
login-usage-consent = Помогите нам оценить, сколько людей пользуется Sonora.
login-device-code = Введите этот код на { $url }
login-server-title = Подключитесь к своему серверу Subsonic
login-server-detail = Введите адрес любого сервера Subsonic или OpenSubsonic (Navidrome, Airsonic, Gonic, …), затем войдите, указав имя пользователя и пароль сервера. Сессия остаётся на этом устройстве.
login-server-hint = https://music.example.com
login-username-hint = Имя пользователя
login-password-hint = Пароль
login-server-submit = Подключиться
login-account-title = Выберите аккаунт
login-account-detail = В этой сессии выполнен вход в несколько аккаунтов Google. Выберите тот, который должна использовать Sonora.
login-choose-title = Вход в { $provider }
login-choose-detail = Выберите способ входа в { $provider }.

# album and playlist pages

# artist page
artist-eyebrow = Исполнитель
artist-monthly-listeners = { $count ->
    [one] { $value } слушатель в месяц
    [few] { $value } слушателя в месяц
   *[other] { $value } слушателей в месяц
}
artist-popular = Популярное
artist-popular-eyebrow = Знакомство с исполнителем
artist-popular-empty = Пока нечего слушать у этого исполнителя
artist-releases = Релизы
artist-filter-all = Все
artist-filter-singles = Синглы
artist-filter-eps = EP
artist-appears-on = Появляется в
album-also-like = Вам также может понравиться
album-tab-artists = Артисты
album-label = ℗ { $label }

# user profile page
user-eyebrow = Профиль
user-followers = { $count ->
    [one] { $value } подписчик
    [few] { $value } подписчика
   *[other] { $value } подписчиков
}
user-following = { $count ->
    [one] { $value } подписка
    [few] { $value } подписки
   *[other] { $value } подписок
}
user-playlists = Публичные плейлисты
user-playlists-empty = Пока нет публичных плейлистов

# home page
home-quick-picks = Быстрый выбор
home-quick-picks-empty = Добавьте несколько треков, и они появятся здесь
home-recently-added = Недавно добавленные
home-recently-played = Недавно прослушанные
home-favorite-albums = Избранные альбомы
home-artists = Исполнители
home-collection-albums = Альбомы из вашей коллекции

# search page
search-placeholder = Что хотите послушать?
search-browse = Всё подряд
search-submit = Найти
search-recent = Недавние запросы
search-forget = Удалить из истории
genre-empty = Здесь пока пусто
search-best-match = Лучшее совпадение
search-no-matches = Ничего не найдено
search-results = Результаты
search-artists = Исполнители
search-albums-playlists = Альбомы и плейлисты
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } трек в медиатеке
        [few] { $count } трека в медиатеке
       *[other] { $count } треков в медиатеке
    }

# song page
song-eyebrow = Трек
song-view-album = Открыть альбом
song-about = О треке
song-album = Альбом
song-released = Дата выхода
song-streams = Прослушивания
song-position = Позиция
song-label = Лейбл
song-popularity = Популярность
song-disc-track = Диск { $disc }, трек { $track }
song-track = Трек { $track }
song-credits = Авторы
song-performed-by = Исполнение
song-details = Жанры и детали
song-genres = Жанры
song-language = Язык
song-content = Контент
song-explicit = Ненормативная лексика
song-local = Локальный файл
song-clean = Без ограничений
artist-about = Об исполнителе
artist-about-fallback = Послушайте популярные треки и релизы исполнителя.
artist-about-open = Перейти к исполнителю
song-copyright = © { $notice }

# tags
tags-edit-title = Изменение тегов
tags-sheet-song = Трек
tags-sheet-album = Альбом
tags-sheet-details = Детали
tags-title = Название
tags-artist = Исполнитель
tags-artists = Исполнители
tags-add-artist = Добавить исполнителя
tags-remove-artist = Удалить этого исполнителя
tags-track = Номер трека
tags-track-total = Треков в релизе
tags-disc = Номер диска
tags-disc-total = Дисков в релизе
tags-album = Альбом
tags-album-artist = Исполнитель альбома
tags-year = Год
tags-genre = Жанр
tags-composer = Композитор
tags-publisher = Издатель
tags-isrc = ISRC
tags-comment = Комментарий
