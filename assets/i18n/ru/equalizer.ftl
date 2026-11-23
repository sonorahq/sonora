equalizer-advanced = Расширенный
equalizer-simple = Простой
equalizer-sound = Звук
equalizer-profile-default = По умолчанию
equalizer-profile-new = Профиль { $number }
equalizer-profile-pasted = Вставленный профиль
equalizer-profile-name-hint = Название профиля
equalizer-save-as = Сохранить как профиль
equalizer-rename = Переименовать профиль
equalizer-delete = Удалить профиль
equalizer-bass = Басы
equalizer-treble = Высокие
equalizer-tone-reset = Сбросить до 0 дБ
equalizer-preamp = Предусиление
equalizer-preamp-auto = Автоматически
equalizer-preamp-manual = Вручную
equalizer-decibels = дБ
equalizer-hertz = Гц
equalizer-q = Q
equalizer-octaves = окт.
equalizer-no-gain = —
equalizer-filters = Фильтры
equalizer-no-filters = Фильтров пока нет. Добавьте фильтр, выберите пресет выше или импортируйте профиль.
equalizer-add-filter = Добавить фильтр
equalizer-remove-filter = Удалить фильтр
equalizer-kind-peak = Пиковый
equalizer-kind-low-shelf = Низкая полка
equalizer-kind-high-shelf = Высокая полка
equalizer-kind-low-pass = Фильтр НЧ
equalizer-kind-high-pass = Фильтр ВЧ
equalizer-kind-notch = Режекторный
equalizer-kind-band-pass = Полосовой
equalizer-kind-all-pass = Всепропускающий
equalizer-transfer = Импорт и экспорт
equalizer-import = Импорт
equalizer-export = Экспорт
equalizer-import-file = Импорт из файла…
equalizer-export-file = Экспорт в файл…
equalizer-imported = Импортировано: { $name }
equalizer-imported-skipped = Импортировано: { $name }, без { $count ->
        [one] { $count } строки
        [few] { $count } строк
       *[other] { $count } строк
    }
equalizer-import-nothing = Не найдено настроек эквалайзера для импорта
equalizer-import-too-many = Не импортировано: { $count ->
        [one] включён { $count } фильтр
        [few] включено { $count } фильтра
       *[other] включено { $count } фильтров
    }, больше, чем вмещает профиль
equalizer-import-too-large = Не импортировано: текст больше 1 МБ, слишком много для профиля эквалайзера
equalizer-filter-limit =В профиле может быть до { $max } фильтров.
equalizer-paste-empty = В буфере обмена нет текста
equalizer-copied = Профиль скопирован как текст Equalizer APO
equalizer-exported = Профиль экспортирован
equalizer-file-unreadable = Не удалось прочитать файл
equalizer-file-unwritable = Не удалось записать файл
equalizer-value-refused = Оставлено прежнее значение
equalizer-skipped-line = Строка { $line }: { $reason }
equalizer-skipped-more = и ещё { $count ->
        [one] { $count } строка
        [few] { $count } строки
       *[other] { $count } строк
    }
equalizer-skip-unknown-type = неизвестный тип фильтра { $name }
equalizer-skip-unreadable = не распознано как фильтр
equalizer-skip-no-frequency = нет частоты
equalizer-skip-no-gain = нет усиления
equalizer-skip-no-width = нет Q или ширины полосы
equalizer-skip-expression = значение записано выражением
equalizer-skip-no-preamp = предусиление не число
equalizer-skip-unsupported = { $name } не поддерживается
equalizer-rules = Правила для устройств
equalizer-rule-other = Любое другое устройство
equalizer-rule-playing = Сейчас играет
equalizer-rule-replaces = Заменяет его правило
equalizer-rule-choose-device = Выберите устройство
equalizer-rule-choose-sound = Выберите звучание
equalizer-rule-add = Добавить правило
equalizer-rule-remove = Удалить правило
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = Частота должна быть от { $low } до { $high } Гц
equalizer-invalid-gain = Усиление должно быть от { $low } до { $high } дБ
equalizer-invalid-q = Q должно быть от { $low } до { $high }
equalizer-invalid-octaves = Ширина полосы должна быть от { $low } до { $high } октав
equalizer-invalid-shelf-octaves = Для полки задаётся Q, а не ширина полосы
equalizer-invalid-preamp = Предусиление должно быть от { $low } до { $high } дБ
equalizer-overload = Клиппинг на { $db } дБ
equalizer-overload-slight = Клиппинг меньше 0,1 дБ
equalizer-lower-preamp = Снизить предусиление
equalizer-level = Уровень
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Bit-perfect на выходе Sonora
equalizer-status-server = Затем звуковой сервер переводит его в { $khz } кГц
equalizer-status-altered = Не bit-perfect из-за:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Громкость
equalizer-status-normalized = Нормализация
equalizer-status-direct = Играет Soloist, мимо Sonora
equalizer-status-rate = { $from } → { $to } кГц
equalizer-status-bits = { $from } → { $to } бит
equalizer-status-float = Float → { $to } бит
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
