# login
login-restoring = Comprobando tu sesión guardada…
login-authorizing = Esperando la autorización en tu navegador…
login-signed-in = Sesión iniciada como { $name }
login-failed-title = No se pudo iniciar sesión
login-problem-region = Spotify no abre una sesión desde el país en el que estás. Inicia sesión desde tu país de origen o cambia el país en tu cuenta de Spotify.
login-problem-credentials = Tu sesión guardada de Spotify ya no es válida. Inicia sesión de nuevo para continuar.
login-problem-network = Sonora no pudo conectarse con el servicio de música. Revisa tu conexión a internet e inténtalo de nuevo.
login-problem-cancelled = Cerraste la página del navegador antes de aprobar el inicio de sesión. Empieza de nuevo para terminar.
login-problem-refused = Spotify rechazó el inicio de sesión. Espera un momento e inténtalo de nuevo.
login-problem-premium = Sonora reproduce a través de Spotify Premium y esta cuenta no lo tiene. Inicia sesión con una cuenta Premium para continuar.
login-sign-in = Iniciar sesión con { $provider }
login-connect-cookies = Pegar las cookies manualmente
login-cookie-open = Abrir { $provider }
login-cookie-submit = Continuar
login-cookie-hint = Pega aquí el encabezado de solicitud Cookie
login-cookie-step-1 = Abre music.youtube.com y comprueba que has iniciado sesión. Funciona mejor en una ventana de incógnito.
login-cookie-step-2 = Pulsa F12, abre la pestaña Red y recarga la página.
login-cookie-step-3 = Selecciona cualquier solicitud llamada "browse" o "next".
login-cookie-step-4 = En Encabezados, busca Cookie dentro de los encabezados de solicitud, haz clic derecho y copia su valor.
login-cookie-step-note = Pega el valor completo abajo: el panel de cookies de la solicitud no basta, porque el valor tiene que llevar SAPISID y __Secure-3PAPISID.
login-cookie-header-title = Pega las cookies de { $provider } para terminar de iniciar sesión
login-cookie-apple-step-3 = Selecciona cualquier solicitud a { $site }.
login-cookie-apple-note = Asegúrate de pegar el valor completo, incluido { $cookie }.
login-cookie-named-title = Pega la cookie de { $provider } para terminar de iniciar sesión
login-cookie-named-hint = Pega aquí el valor de la cookie
login-cookie-named-step-1 = Abre { $site } y comprueba que has iniciado sesión.
login-cookie-named-step-2 = Pulsa F12 y abre la pestaña Almacenamiento en Firefox, o la pestaña Aplicación en Chrome.
login-cookie-named-step-3 = Despliega Cookies, selecciona { $site } y busca la cookie llamada { $cookie }.
login-cookie-named-step-4 = Haz doble clic en su valor y cópialo.
login-cookie-named-note = Basta con el valor de { $cookie }. También sirve un encabezado Cookie completo copiado desde la pestaña Red.
login-guest-title = Modo invitado
login-guest-use = Usar el modo invitado
login-usage-consent = Ayúdanos a estimar cuánta gente usa Sonora.
login-device-code = Introduce este código en { $url }
login-server-title = Conéctate a tu servidor Subsonic
login-server-detail = Introduce la dirección de cualquier servidor Subsonic u OpenSubsonic (Navidrome, Airsonic, Gonic, …) e inicia sesión con tu nombre de usuario y contraseña del servidor. La sesión permanece en este dispositivo.
login-server-hint = https://music.example.com
login-username-hint = Nombre de usuario
login-password-hint = Contraseña
login-server-submit = Conectar
login-account-title = Elige una cuenta
login-account-detail = Esta sesión tiene iniciada la sesión en más de una cuenta de Google. Elige la que debe usar Sonora.
login-choose-title = Iniciar sesión en { $provider }
login-choose-detail = Elige cómo iniciar sesión en { $provider }.

# album and playlist pages

# artist page
artist-eyebrow = Artista
artist-monthly-listeners = { $count ->
    [one] { $value } oyente mensual
   *[other] { $value } oyentes mensuales
}
artist-popular = Popular
artist-popular-eyebrow = Descubre a este artista
artist-popular-empty = Todavía no hay nada que reproducir de este artista
artist-releases = Lanzamientos
artist-filter-all = Todo
artist-filter-singles = Sencillos
artist-filter-eps = EP
artist-appears-on = Aparece en
album-also-like = También te puede gustar
album-tab-artists = Artistas
album-label = ℗ { $label }

# user profile page
user-eyebrow = Perfil
user-followers = { $count ->
    [one] { $value } seguidor
   *[other] { $value } seguidores
}
user-following = { $count ->
   *[other] { $value } siguiendo
}
user-playlists = Listas públicas
user-playlists-empty = Todavía no hay listas públicas

# home page
home-quick-picks = Selección rápida
home-quick-picks-empty = Marca algunas canciones como favoritas y aparecerán aquí
home-recently-added = Añadidos recientemente
home-recently-played = Reproducidos recientemente
home-favorite-albums = Álbumes favoritos
home-artists = Artistas
home-collection-albums = Álbumes de tu colección

# search page
search-placeholder = ¿Qué quieres escuchar?
search-browse = Explorar todo
search-submit = Buscar
search-recent = Búsquedas recientes
search-forget = Quitar del historial
genre-empty = Todavía no hay nada que mostrar aquí
search-best-match = Mejor coincidencia
search-no-matches = Sin coincidencias
search-results = Resultados
search-artists = Artistas
search-albums-playlists = Álbumes y listas
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } canción en la biblioteca
       *[other] { $count } canciones en la biblioteca
    }

# song page
song-eyebrow = Canción
song-view-album = Ver álbum
song-about = Sobre esta canción
song-album = Álbum
song-released = Lanzamiento
song-streams = Reproducciones
song-position = Posición
song-label = Sello
song-popularity = Popularidad
song-disc-track = Disco { $disc }, pista { $track }
song-track = Pista { $track }
song-credits = Créditos
song-performed-by = Interpretada por
song-details = Géneros y detalles
song-genres = Géneros
song-language = Idioma
song-content = Contenido
song-explicit = Explícito
song-clean = Sin contenido explícito
artist-about = Sobre el artista
artist-about-fallback = Descubre las canciones populares y los lanzamientos del artista.
artist-about-open = Ir al artista
song-copyright = © { $notice }

# tags
tags-edit-title = Editar etiquetas
tags-sheet-song = Canción
tags-sheet-album = Álbum
tags-sheet-details = Detalles
tags-title = Título
tags-artist = Artista
tags-artists = Artistas
tags-add-artist = Añadir un artista
tags-remove-artist = Quitar este artista
tags-track = Número de pista
tags-track-total = Pistas del lanzamiento
tags-disc = Número de disco
tags-disc-total = Discos del lanzamiento
tags-album = Álbum
tags-album-artist = Artista del álbum
tags-year = Año
tags-genre = Género
tags-composer = Compositor
tags-publisher = Editora
tags-isrc = ISRC
tags-comment = Comentario
