# track menu
menu-new-playlist = Новый плейлист
menu-import-playlist = Импортировать плейлист…
menu-no-playlists = Нет плейлистов
menu-search-playlists = Поиск плейлиста
menu-save-library = Медиатека
menu-add-to-library = В избранное
menu-remove-from-library = Убрать из избранного
menu-remove-tracks-from-playlist = { $count ->
    [one] Удалить { $count } трек из плейлиста
    [few] Удалить { $count } трека из плейлиста
   *[other] Удалить { $count } треков из плейлиста
}
menu-remove-tracks-from-history = { $count ->
    [one] Удалить { $count } трек из истории
    [few] Удалить { $count } трека из истории
   *[other] Удалить { $count } треков из истории
}
menu-download = Скачать
menu-remove-download = Удалить загрузку
menu-downloading = Скачивается. Щёлкните для остановки
menu-song-radio = Радио по треку
menu-go-to-album = Перейти к альбому
menu-go-to-artist = Перейти к исполнителю
menu-view-details = Подробнее
menu-copy-link = Копировать ссылку
menu-select-all = Выделить всё
menu-remove-from-queue = Убрать из очереди
menu-export-playlist = Экспортировать плейлист…
menu-add-playlist-to-library = Добавить в медиатеку
menu-remove-playlist-from-library = Удалить из медиатеки
menu-make-playlist-public = Сделать публичным
menu-make-playlist-private = Сделать приватным
menu-play-artist = Играть исполнителя

# playlist editor
menu-copy-link-none = Здесь нет ссылки, которой можно поделиться
menu-copy-link-local = У локальных исполнителей и плейлистов нет ссылок
menu-play-next = { $count ->
    [1] Играть следующим
    [one] Играть { $count } трек следующим
    [few] Играть { $count } трека следующими
   *[other] Играть { $count } треков следующими
}
menu-add-to-queue = { $count ->
    [1] Добавить в очередь
    [one] Добавить { $count } трек в очередь
    [few] Добавить { $count } трека в очередь
   *[other] Добавить { $count } треков в очередь
}
menu-play-last = { $count ->
    [1] Играть в конце
    [one] Играть { $count } трек в конце
    [few] Играть { $count } трека в конце
   *[other] Играть { $count } треков в конце
}
menu-delete-track-files = { $count ->
    [1] Удалить файл трека
   *[other] Удалить файлы треков ({ $count })
}
menu-add-to-playlist = { $count ->
    [1] Добавить в плейлист
    [one] Добавить { $count } трек в плейлист
    [few] Добавить { $count } трека в плейлист
   *[other] Добавить { $count } треков в плейлист
}

# playlist editor
playlist-name-placeholder = Название плейлиста
playlist-create-title = Создать плейлист
playlist-create-shelf = Сохранить в
playlist-edit-title = Редактировать плейлист
playlist-cover-choose = Выбрать картинку
playlist-cover-remove = Убрать обложку
playlist-cover-unopenable = Sonora не может открыть этот файл
playlist-cover-too-large = Картинка больше 40 МБ
playlist-cover-not-a-picture = Это не картинка в JPEG, PNG или WebP
playlist-delete-title = Удалить плейлист
playlist-again-title = Добавить ещё раз?
playlist-again-confirm = Этот трек уже есть в «{ $name }». Добавить копию?
playlist-again-add = Добавить ещё раз

# confirm

# confirm
confirm-delete-track-files-title = Удалить файлы треков?
confirm-delete-track-files = Файлы будут безвозвратно удалены с диска. Это действие нельзя отменить.
confirm-uninstall-widevine-title = Удалить модуль Widevine?
confirm-uninstall-widevine = Sonora удалит копию, скачанную у Google. Треки Apple Music перестанут играть, пока модуль не будет скачан снова.
confirm-remove-soloist-title = Удалить Soloist?
confirm-remove-soloist = Sonora удалит программу Soloist, ваш API-ключ и её вход в аккаунт. Spotify вернётся на встроенный плеер.
confirm-remove-downloads-title = Удалить все загрузки?
confirm-remove-downloads = Все скачанные треки из всех сервисов будут удалены с этого компьютера. Они снова будут играть по сети, пока вы не скачаете их заново.
confirm-delete-named = Удалить «{ $name }»? Это действие нельзя отменить.
