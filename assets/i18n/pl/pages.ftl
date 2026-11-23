# login
login-restoring = Sprawdzanie zapisanej sesji…
login-authorizing = Oczekiwanie na autoryzację w przeglądarce…
login-signed-in = Zalogowano jako { $name }
login-failed-title = Logowanie nie powiodło się
login-problem-region = Spotify nie otworzy sesji z kraju, w którym jesteś. Zaloguj się z kraju macierzystego albo zmień kraj na koncie Spotify.
login-problem-credentials = Zapisana sesja Spotify straciła ważność. Zaloguj się ponownie.
login-problem-network = Sonora nie mogła połączyć się z serwisem muzycznym. Sprawdź połączenie internetowe i spróbuj ponownie.
login-problem-cancelled = Zamknięto stronę przeglądarki przed zatwierdzeniem logowania. Zacznij od nowa.
login-problem-refused = Spotify odrzucił logowanie. Odczekaj chwilę i spróbuj ponownie.
login-problem-premium = Sonora odtwarza muzykę przez Spotify Premium, a to konto go nie ma. Zaloguj się na konto z Premium, aby kontynuować.
login-sign-in = Zaloguj się przez { $provider }
login-connect-cookies = Wklej pliki cookie ręcznie
login-cookie-open = Otwórz { $provider }
login-cookie-submit = Kontynuuj
login-cookie-hint = Wklej tutaj nagłówek żądania Cookie
login-cookie-step-1 = Otwórz music.youtube.com i upewnij się, że jesteś zalogowany. Najlepiej działa okno incognito.
login-cookie-step-2 = Naciśnij F12, otwórz kartę Sieć i odśwież stronę.
login-cookie-step-3 = Wybierz dowolne żądanie o nazwie „browse” lub „next”.
login-cookie-step-4 = Na karcie Nagłówki znajdź Cookie wśród nagłówków żądania, kliknij prawym przyciskiem i skopiuj wartość.
login-cookie-step-note = Wklej całą wartość: panel Ciasteczka nie wystarczy, bo wartość musi zawierać SAPISID i __Secure-3PAPISID.
login-cookie-header-title = Wklej pliki cookie { $provider }, aby dokończyć logowanie
login-cookie-apple-step-3 = Wybierz dowolne żądanie do { $site }.
login-cookie-apple-note = Wklej całą wartość: musi zawierać { $cookie }.
login-cookie-named-title = Wklej plik cookie { $provider }, aby dokończyć logowanie
login-cookie-named-hint = Wklej tutaj wartość pliku cookie
login-cookie-named-step-1 = Otwórz { $site } i upewnij się, że jesteś zalogowany.
login-cookie-named-step-2 = Naciśnij F12 i otwórz kartę Dane w Firefoksie lub Application w Chrome.
login-cookie-named-step-3 = Rozwiń Cookies, wybierz { $site } i znajdź plik cookie o nazwie { $cookie }.
login-cookie-named-step-4 = Kliknij dwukrotnie jego wartość i skopiuj ją.
login-cookie-named-note = Sama wartość { $cookie } wystarczy. Cały nagłówek Cookie skopiowany z karty Sieć też zadziała.
login-guest-title = Tryb gościa
login-guest-use = Otwórz tryb gościa
login-usage-consent = Pomóż nam oszacować, ile osób korzysta z Sonory.
login-device-code = Wpisz ten kod na stronie { $url }
login-server-title = Połącz się ze swoim serwerem Subsonic
login-server-detail = Wpisz adres dowolnego serwera Subsonic lub OpenSubsonic (Navidrome, Airsonic, Gonic, …), a następnie zaloguj się przy użyciu nazwy użytkownika i hasła serwera. Sesja pozostaje na tym urządzeniu.
login-server-hint = https://music.example.com
login-username-hint = Nazwa użytkownika
login-password-hint = Hasło
login-server-submit = Połącz
login-account-title = Wybierz konto
login-account-detail = W tej sesji zalogowano więcej niż jedno konto Google. Wybierz to, którego ma używać Sonora.
login-choose-title = Zaloguj się do { $provider }
login-choose-detail = Wybierz sposób logowania do { $provider }.

# album and playlist pages

# artist page
artist-eyebrow = Wykonawca
artist-monthly-listeners = { $count ->
    [one] { $value } słuchacz miesięcznie
    [few] { $value } słuchaczy miesięcznie
   *[other] { $value } słuchaczy miesięcznie
}
artist-popular = Popularne
artist-popular-eyebrow = Poznaj tego wykonawcę
artist-popular-empty = Nie ma jeszcze czego odtworzyć u tego wykonawcy
artist-releases = Wydawnictwa
artist-filter-all = Wszystkie
artist-filter-singles = Single
artist-filter-eps = EP
artist-appears-on = Występuje w
album-also-like = Może Ci się spodobać
album-tab-artists = Artyści
album-label = ℗ { $label }

# user profile page
user-eyebrow = Profil
user-followers = { $count ->
    [one] { $value } obserwujący
    [few] { $value } obserwujących
   *[other] { $value } obserwujących
}
user-following = { $count ->
    [one] { $value } obserwowany
    [few] { $value } obserwowanych
   *[other] { $value } obserwowanych
}
user-playlists = Publiczne playlisty
user-playlists-empty = Brak publicznych playlist

# home page
home-quick-picks = Szybki wybór
home-quick-picks-empty = Polub kilka utworów, a pojawią się tutaj
home-recently-added = Ostatnio dodane
home-recently-played = Ostatnio odtwarzane
home-favorite-albums = Ulubione albumy
home-artists = Wykonawcy
home-collection-albums = Albumy z Twojej kolekcji

# search page
search-placeholder = Czego chcesz posłuchać?
search-browse = Przeglądaj
search-submit = Szukaj
search-recent = Ostatnie wyszukiwania
search-forget = Usuń z historii
genre-empty = Nie ma tu jeszcze nic
search-best-match = Najlepszy wynik
search-no-matches = Brak wyników
search-results = Wyniki
search-artists = Wykonawcy
search-albums-playlists = Albumy i playlisty
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } utwór w bibliotece
        [few] { $count } utwory w bibliotece
       *[other] { $count } utworów w bibliotece
    }

# song page
song-eyebrow = Utwór
song-view-album = Zobacz album
song-about = O utworze
song-album = Album
song-released = Data wydania
song-streams = Odtworzenia
song-position = Pozycja
song-label = Wytwórnia
song-popularity = Popularność
song-disc-track = Płyta { $disc }, utwór { $track }
song-track = Utwór { $track }
song-credits = Twórcy
song-performed-by = Wykonanie
song-details = Gatunki i szczegóły
song-genres = Gatunki
song-language = Język
song-content = Treść
song-explicit = Wulgaryzmy
song-clean = Bez wulgaryzmów
artist-about = O wykonawcy
artist-about-fallback = Poznaj popularne utwory i wydawnictwa tego wykonawcy.
artist-about-open = Przejdź do wykonawcy
song-copyright = © { $notice }

# tags
tags-edit-title = Edycja tagów
tags-sheet-song = Utwór
tags-sheet-album = Album
tags-sheet-details = Szczegóły
tags-title = Tytuł
tags-artist = Wykonawca
tags-artists = Wykonawcy
tags-add-artist = Dodaj wykonawcę
tags-remove-artist = Usuń tego wykonawcę
tags-track = Numer utworu
tags-track-total = Utworów w wydaniu
tags-disc = Numer płyty
tags-disc-total = Płyt w wydaniu
tags-album = Album
tags-album-artist = Wykonawca albumu
tags-year = Rok
tags-genre = Gatunek
tags-composer = Kompozytor
tags-publisher = Wydawca
tags-isrc = ISRC
tags-comment = Komentarz
