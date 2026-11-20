# track menu
menu-new-playlist = Nowa playlista
menu-import-playlist = Importuj playlistę…
menu-no-playlists = Brak playlist
menu-search-playlists = Szukaj playlisty
menu-save-library = Biblioteka
menu-add-to-library = Dodaj do ulubionych
menu-remove-from-library = Usuń z ulubionych
menu-remove-tracks-from-playlist = { $count ->
    [one] Usuń { $count } utwór z playlisty
    [few] Usuń { $count } utwory z playlisty
   *[other] Usuń { $count } utworów z playlisty
}
menu-remove-tracks-from-history = { $count ->
    [one] Usuń { $count } utwór z historii
    [few] Usuń { $count } utwory z historii
   *[other] Usuń { $count } utworów z historii
}
menu-download = Pobierz
menu-remove-download = Usuń z pobranych
menu-downloading = Pobieranie. Kliknij, aby zatrzymać
menu-song-radio = Radio utworu
menu-go-to-album = Przejdź do albumu
menu-go-to-artist = Przejdź do wykonawcy
menu-view-details = Szczegóły
menu-copy-link = Kopiuj link
menu-select-all = Zaznacz wszystko
menu-remove-from-queue = Usuń z kolejki
menu-export-playlist = Eksportuj playlistę…
menu-add-playlist-to-library = Dodaj do biblioteki
menu-remove-playlist-from-library = Usuń z biblioteki
menu-make-playlist-public = Ustaw jako publiczną
menu-make-playlist-private = Ustaw jako prywatną
menu-play-artist = Odtwórz wykonawcę

# playlist editor
menu-copy-link-none = Brak linku do udostępnienia
menu-copy-link-local = Lokalni wykonawcy i playlisty nie mają linków
menu-play-next = { $count ->
    [1] Odtwórz jako następny
    [one] Odtwórz { $count } utwór jako następny
    [few] Odtwórz { $count } utwory jako następne
   *[other] Odtwórz { $count } utworów jako następne
}
menu-add-to-queue = { $count ->
    [1] Dodaj do kolejki
    [one] Dodaj { $count } utwór do kolejki
    [few] Dodaj { $count } utwory do kolejki
   *[other] Dodaj { $count } utworów do kolejki
}
menu-play-last = { $count ->
    [1] Odtwórz na końcu
    [one] Odtwórz { $count } utwór na końcu
    [few] Odtwórz { $count } utwory na końcu
   *[other] Odtwórz { $count } utworów na końcu
}
menu-delete-track-files = { $count ->
    [1] Usuń plik utworu
   *[other] Usuń pliki utworów ({ $count })
}

# playlist editor
playlist-name-placeholder = Nazwa playlisty
playlist-create-title = Utwórz playlistę
playlist-edit-title = Edytuj playlistę
playlist-cover-choose = Wybierz obraz
playlist-cover-remove = Usuń okładkę
playlist-cover-unopenable = Sonora nie może otworzyć tego pliku
playlist-cover-too-large = Ten obraz ma ponad 40 MB
playlist-cover-not-a-picture = Ten plik nie jest obrazem JPEG, PNG ani WebP
playlist-delete-title = Usuń playlistę
playlist-again-title = Dodać ponownie?
playlist-again-confirm = Ten utwór jest już w „{ $name }”. Dodać kopię?
playlist-again-add = Dodaj ponownie

# confirm

# confirm
confirm-delete-track-files-title = Usunąć pliki utworów?
confirm-delete-track-files = Pliki zostaną trwale usunięte z dysku. Nie można tego cofnąć.
confirm-uninstall-widevine-title = Odinstalować moduł Widevine?
confirm-uninstall-widevine = Sonora usunie kopię pobraną od Google. Utwory Apple Music przestaną grać, dopóki moduł nie zostanie pobrany ponownie.
confirm-remove-downloads-title = Usunąć wszystkie pobrane?
confirm-remove-downloads = Wszystkie pobrane utwory ze wszystkich serwisów zostaną usunięte z tego komputera. Będą znów streamowane, dopóki ich ponownie nie pobierzesz.
confirm-delete-named = Usunąć „{ $name }”? Tej operacji nie można cofnąć.
