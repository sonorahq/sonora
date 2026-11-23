equalizer-advanced = Zaawansowany
equalizer-simple = Prosty
equalizer-sound = Brzmienie
equalizer-profile-default = Domyślny
equalizer-profile-new = Profil { $number }
equalizer-profile-pasted = Wklejony profil
equalizer-profile-name-hint = Nazwa profilu
equalizer-save-as = Zapisz jako profil
equalizer-rename = Zmień nazwę profilu
equalizer-delete = Usuń profil
equalizer-bass = Bas
equalizer-treble = Soprany
equalizer-tone-reset = Przywróć 0 dB
equalizer-preamp = Przedwzmacniacz
equalizer-preamp-auto = Automatyczny
equalizer-preamp-manual = Ręczny
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = okt.
equalizer-no-gain = —
equalizer-filters = Filtry
equalizer-no-filters = Brak filtrów. Dodaj filtr, wybierz preset powyżej lub zaimportuj profil.
equalizer-add-filter = Dodaj filtr
equalizer-remove-filter = Usuń filtr
equalizer-kind-peak = Szczytowy
equalizer-kind-low-shelf = Półkowy niski
equalizer-kind-high-shelf = Półkowy wysoki
equalizer-kind-low-pass = Dolnoprzepustowy
equalizer-kind-high-pass = Górnoprzepustowy
equalizer-kind-notch = Wycinający
equalizer-kind-band-pass = Pasmowoprzepustowy
equalizer-kind-all-pass = Wszechprzepustowy
equalizer-transfer = Import i eksport
equalizer-import = Importuj
equalizer-export = Eksportuj
equalizer-import-file = Importuj plik…
equalizer-export-file = Eksportuj plik…
equalizer-imported = Zaimportowano { $name }
equalizer-imported-skipped = Zaimportowano { $name } bez { $count ->
        [one] jednego wiersza
        [few] { $count } wierszy
       *[other] { $count } wierszy
    }
equalizer-import-nothing = Nie znaleziono ustawień korektora do zaimportowania
equalizer-import-too-many = Nie zaimportowano: { $count ->
        [one] włączony jest jeden filtr
        [few] włączone są { $count } filtry
       *[other] włączonych jest { $count } filtrów
    }, więcej niż mieści profil
equalizer-import-too-large = Nie zaimportowano: tekst ma ponad 1 MB, za dużo jak na profil korektora
equalizer-filter-limit =Profil mieści do { $max } filtrów.
equalizer-paste-empty = Schowek nie zawiera tekstu
equalizer-copied = Skopiowano profil jako tekst Equalizer APO
equalizer-exported = Wyeksportowano profil
equalizer-file-unreadable = Nie udało się odczytać pliku
equalizer-file-unwritable = Nie udało się zapisać pliku
equalizer-value-refused = Zachowano poprzednią wartość
equalizer-skipped-line = Wiersz { $line }: { $reason }
equalizer-skipped-more = i { $count ->
        [one] jeszcze jeden
        [few] jeszcze { $count }
       *[other] jeszcze { $count }
    }
equalizer-skip-unknown-type = nieznany typ filtra { $name }
equalizer-skip-unreadable = to nie jest filtr, który Sonora potrafi odczytać
equalizer-skip-no-frequency = brak częstotliwości
equalizer-skip-no-gain = brak wzmocnienia
equalizer-skip-no-width = brak Q ani szerokości pasma
equalizer-skip-expression = wartość zapisana jako wyrażenie
equalizer-skip-no-preamp = przedwzmacniacz nie jest liczbą
equalizer-skip-unsupported = { $name } nie jest obsługiwany
equalizer-rules = Reguły urządzeń
equalizer-rule-other = Każde inne urządzenie
equalizer-rule-playing = Teraz gra
equalizer-rule-replaces = Zastępuje jego regułę
equalizer-rule-choose-device = Wybierz urządzenie
equalizer-rule-choose-sound = Wybierz brzmienie
equalizer-rule-add = Dodaj regułę
equalizer-rule-remove = Usuń regułę
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = Częstotliwość musi wynosić od { $low } do { $high } Hz
equalizer-invalid-gain = Wzmocnienie musi wynosić od { $low } do { $high } dB
equalizer-invalid-q = Q musi wynosić od { $low } do { $high }
equalizer-invalid-octaves = Szerokość pasma musi wynosić od { $low } do { $high } oktaw
equalizer-invalid-shelf-octaves = Filtr półkowy przyjmuje Q, a nie szerokość pasma
equalizer-invalid-preamp = Przedwzmacniacz musi wynosić od { $low } do { $high } dB
equalizer-overload = Przesterowanie o { $db } dB
equalizer-overload-slight = Przesterowanie poniżej 0,1 dB
equalizer-lower-preamp = Obniż przedwzmacniacz
equalizer-level = Poziom
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Bit-perfect z Sonory
equalizer-status-server = Serwer dźwięku konwertuje go potem do { $khz } kHz
equalizer-status-altered = Brak bit-perfect z powodu:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Głośność
equalizer-status-normalized = Normalizacja
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bit
equalizer-status-float = Float → { $to } bit
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Stereo
   *[other] { $from } kan.
} → { $to ->
    [1] mono
    [2] stereo
   *[other] { $to } kan.
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB
