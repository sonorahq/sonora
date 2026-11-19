equalizer-advanced = Розширений
equalizer-simple = Простий
equalizer-sound = Звук
equalizer-profile-default = За замовчуванням
equalizer-profile-new = Профіль { $number }
equalizer-profile-pasted = Вставлений профіль
equalizer-profile-name-hint = Назва профілю
equalizer-save-as = Зберегти як профіль
equalizer-rename = Перейменувати профіль
equalizer-delete = Видалити профіль
equalizer-bass = Баси
equalizer-treble = Високі
equalizer-tone-reset = Скинути до 0 дБ
equalizer-preamp = Передпідсилення
equalizer-preamp-auto = Автоматично
equalizer-preamp-manual = Вручну
equalizer-decibels = дБ
equalizer-hertz = Гц
equalizer-q = Q
equalizer-octaves = окт
equalizer-no-gain = —
equalizer-filters = Фільтри
equalizer-no-filters = Фільтрів ще немає. Додайте фільтр, виберіть пресет вище або імпортуйте профіль.
equalizer-add-filter = Додати фільтр
equalizer-remove-filter = Видалити фільтр
equalizer-kind-peak = Піковий
equalizer-kind-low-shelf = Полиця НЧ
equalizer-kind-high-shelf = Полиця ВЧ
equalizer-kind-low-pass = Фільтр НЧ
equalizer-kind-high-pass = Фільтр ВЧ
equalizer-kind-notch = Режекторний
equalizer-kind-band-pass = Смуговий
equalizer-kind-all-pass = Фазовий
equalizer-transfer = Імпорт і експорт
equalizer-import = Імпорт
equalizer-export = Експорт
equalizer-import-file = Імпортувати файл…
equalizer-export-file = Експортувати файл…
equalizer-imported = Імпортовано { $name }
equalizer-imported-skipped = Імпортовано { $name } без { $count ->
        [one] { $count } рядка
        [few] { $count } рядків
       *[other] { $count } рядків
    }
equalizer-import-nothing = Не знайдено налаштувань еквалайзера для імпорту
equalizer-import-too-many = Не імпортовано: { $count ->
        [one] увімкнено { $count } фільтр
        [few] увімкнено { $count } фільтри
       *[other] увімкнено { $count } фільтрів
    }, більше, ніж вміщує профіль
equalizer-import-too-large = Не імпортовано: текст понад 1 МБ, завеликий для профілю еквалайзера
equalizer-filter-limit =Профіль вміщує до { $max } фільтрів.
equalizer-paste-empty = У буфері обміну немає тексту
equalizer-copied = Профіль скопійовано як текст Equalizer APO
equalizer-exported = Профіль експортовано
equalizer-file-unreadable = Не вдалося прочитати файл
equalizer-file-unwritable = Не вдалося записати файл
equalizer-value-refused = Залишено попереднє значення
equalizer-skipped-line = Рядок { $line }: { $reason }
equalizer-skipped-more = і ще { $count ->
        [one] { $count }
       *[other] { $count }
    }
equalizer-skip-unknown-type = невідомий тип фільтра { $name }
equalizer-skip-unreadable = фільтр, який Sonora не може прочитати
equalizer-skip-no-frequency = немає частоти
equalizer-skip-no-gain = немає підсилення
equalizer-skip-no-width = немає Q або ширини смуги
equalizer-skip-expression = значення записано виразом
equalizer-skip-no-preamp = передпідсилення не є числом
equalizer-skip-unsupported = { $name } не підтримується
equalizer-rules = Правила пристроїв
equalizer-rule-other = Будь-який інший пристрій
equalizer-rule-playing = Грає зараз
equalizer-rule-replaces = Замінює його правило
equalizer-rule-choose-device = Виберіть пристрій
equalizer-rule-choose-sound = Виберіть звук
equalizer-rule-add = Додати правило
equalizer-rule-remove = Видалити правило
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = Частота має бути від { $low } до { $high } Гц
equalizer-invalid-gain = Підсилення має бути від { $low } до { $high } дБ
equalizer-invalid-q = Q має бути від { $low } до { $high }
equalizer-invalid-octaves = Ширина смуги має бути від { $low } до { $high } октав
equalizer-invalid-shelf-octaves = Полиця задається через Q, а не ширину смуги
equalizer-invalid-preamp = Передпідсилення має бути від { $low } до { $high } дБ
equalizer-overload = Перевантаження на { $db } дБ
equalizer-overload-slight = Перевантаження менш ніж на 0,1 дБ
equalizer-lower-preamp = Знизити передпідсилення
equalizer-level = Рівень
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Sonora передає сигнал bit-perfect
equalizer-status-server = Далі звуковий сервер перетворює його на { $khz } кГц
equalizer-status-altered = Не bit-perfect через:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Гучність
equalizer-status-normalized = Нормалізація
equalizer-status-rate = { $from } → { $to } кГц
equalizer-status-bits = { $from } → { $to } біт
equalizer-status-float = Float → { $to } біт
equalizer-status-channels = { $from ->
    [1] Моно
    [2] Стерео
   *[other] { $from } кан.
} → { $to ->
    [1] моно
    [2] стерео
   *[other] { $to } кан.
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } дБ
