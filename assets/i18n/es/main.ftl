# common
common-on = Activado
common-off = Desactivado
common-left = Izquierda
common-right = Derecha
common-search = Buscar
common-unknown = Desconocido
common-not-provided = No proporcionado
common-not-available = No disponible
common-cancel = Cancelar
common-save = Guardar
common-delete = Eliminar
common-play = Reproducir
common-more = Más
common-previous = Anterior
common-next = Siguiente
common-dismiss = Descartar
common-clear = Vaciar
number-group = { "\u00A0" }

# navigation
nav-history = Historial
nav-home = Inicio
nav-search = Buscar
nav-library = Tu biblioteca
nav-settings = Ajustes
nav-songs = Canciones
nav-favorites = Favoritos
nav-albums = Álbumes
nav-playlists = Listas de reproducción
nav-artists = Artistas
nav-local = Música local
nav-back = Atrás
nav-forward = Adelante
nav-sidebar = Mostrar u ocultar la barra lateral
nav-sidebar-right = Mostrar u ocultar la letra y la cola
nav-pinned = Fijados
nav-unpin = Dejar de fijar
nav-pin-hint = Suelta aquí para fijar
library-liked-songs = Favoritos
library-play-liked-songs = Reproducir
library-no-songs = Todavía no tienes favoritos
library-no-albums = Todavía no tienes álbumes guardados
library-no-playlists = Todavía no tienes listas de reproducción
library-no-artists = Todavía no sigues a ningún artista
library-no-local-songs = No se encontraron canciones importadas
library-no-local-favorites = Todavía no tienes favoritos locales
library-no-local-albums = No se encontraron álbumes importados
library-no-local-artists = No se encontraron artistas importados
library-no-local-playlists = Todavía no tienes listas locales
library-no-matches = Sin coincidencias
library-not-loaded = Tu biblioteca no se cargó
library-part-not-loaded = Esta parte de tu biblioteca no se cargó
library-local-unconfigured = Configura tu biblioteca local

# app menu
app-refresh-library = Actualizar biblioteca
app-sign-out = Cerrar sesión
app-quit = Salir

# table columns
column-played-at = Reproducido
column-index = N.º
column-title = Título
column-artist = Artista
column-album = Álbum
column-date-added = Fecha de adición
column-added-by = Añadido por
column-modified = Modificado
column-length = Duración
column-plays = Reproducciones
column-name = Nombre
column-owner = Propietario
column-year = Año
column-tracks = Pistas

# track menu
menu-add-to-playlist = Añadir a una lista
menu-add-tracks-to-playlist = { $count ->
    [one] Añadir { $count } pista a una lista
   *[other] Añadir { $count } pistas a una lista
}
menu-new-playlist = Nueva lista de reproducción
menu-edit-tags = Editar etiquetas
menu-no-playlists = Sin listas de reproducción
menu-add-to-library = Añadir a Favoritos
menu-add-tracks-to-library = { $count ->
    [one] Añadir { $count } pista a Favoritos
   *[other] Añadir { $count } pistas a Favoritos
}
menu-remove-from-library = Quitar de Favoritos
menu-remove-tracks-from-library = { $count ->
    [one] Quitar { $count } pista de Favoritos
   *[other] Quitar { $count } pistas de Favoritos
}
menu-remove-from-playlist = Quitar de la lista
menu-remove-tracks-from-playlist = { $count ->
    [one] Quitar { $count } pista de la lista
   *[other] Quitar { $count } pistas de la lista
}
menu-remove-from-history = Quitar del historial
menu-remove-tracks-from-history = { $count ->
    [one] Quitar { $count } pista del historial
   *[other] Quitar { $count } pistas del historial
}
menu-play-next = Reproducir a continuación
menu-play-tracks-next = { $count ->
    [one] Reproducir { $count } pista a continuación
   *[other] Reproducir { $count } pistas a continuación
}
menu-add-to-queue = Añadir a la cola
menu-add-tracks-to-queue = { $count ->
    [one] Añadir { $count } pista a la cola
   *[other] Añadir { $count } pistas a la cola
}
menu-song-radio = Ir a la radio de la canción
menu-go-to-album = Ir al álbum
menu-go-to-artist = Ir al artista
menu-view-details = Ver detalles
menu-copy-link = Copiar enlace
menu-cut = Cortar
menu-copy = Copiar
menu-paste = Pegar
menu-select-all = Seleccionar todo
menu-remove-from-queue = Quitar de la cola
menu-open-playlist = Abrir lista de reproducción
menu-play-playlist = Reproducir lista
menu-rename-playlist = Renombrar lista
menu-delete-playlist = Eliminar lista
menu-add-playlist-to-library = Añadir a la biblioteca
menu-remove-playlist-from-library = Quitar de la biblioteca
menu-make-playlist-public = Hacer pública
menu-make-playlist-private = Hacer privada
menu-open-album = Abrir álbum
menu-play-album = Reproducir álbum
menu-play-artist = Reproducir artista

# playlist editor
playlist-name-placeholder = Nombre de la lista
playlist-create-title = Crear lista de reproducción
playlist-rename-title = Renombrar lista de reproducción
playlist-delete-title = Eliminar lista de reproducción
playlist-delete-confirm = ¿Eliminar “{ $name }”? Esta acción no se puede deshacer.
playlist-again-title = ¿Añadirla otra vez?
playlist-again-confirm = Esta pista ya está en “{ $name }”. ¿Quieres añadir otra copia?
playlist-again-add = Añadir otra vez

# confirm
confirm-remove-library-title = Quitar de la biblioteca
confirm-remove-playlist-title = Quitar de la lista
confirm-remove-history-title = Quitar del historial
confirm-unfollow-title = Dejar de seguir
confirm-remove-songs = { $count ->
    [one] ¿Quitar esta canción de tu biblioteca?
   *[other] ¿Quitar { $count } canciones de tu biblioteca?
}
confirm-remove-playlist-songs = { $count ->
    [one] ¿Quitar esta canción de la lista?
   *[other] ¿Quitar { $count } canciones de la lista?
}
confirm-remove-history-songs = { $count ->
    [one] ¿Quitar esta canción del historial de reproducción?
   *[other] ¿Quitar { $count } canciones del historial de reproducción?
}
confirm-remove-albums = { $count ->
    [one] ¿Quitar este álbum de tu biblioteca?
   *[other] ¿Quitar { $count } álbumes de tu biblioteca?
}
confirm-unfollow-artists = { $count ->
    [one] ¿Dejar de seguir a este artista?
   *[other] ¿Dejar de seguir a { $count } artistas?
}
confirm-remove-playlists = { $count ->
    [one] ¿Quitar esta lista de tu biblioteca?
   *[other] ¿Quitar { $count } listas de tu biblioteca?
}

# queue panel
queue-title = Cola
queue-history = Historial
queue-now-playing = Sonando ahora
queue-from = De
queue-up-next = A continuación
queue-reset = Restablecer
queue-clear = Vaciar
queue-empty = Tu cola está vacía
queue-similar = Pistas similares
queue-radio = Reproducir pistas similares automáticamente

# player bar
player-nothing-playing = No se está reproduciendo nada
player-percent = { $value } %
player-shuffle = Aleatorio
player-repeat = Repetir
player-repeat-all = Repetir todo
player-repeat-one = Repetir una
player-mute = Silenciar
player-unmute = Activar el sonido
player-previous = Pista anterior
player-next = Pista siguiente
player-fullscreen = Pantalla completa
player-fullscreen-leave = Salir de pantalla completa
fullscreen-artwork = Portada

# filters
filter-history = Filtrar el historial de reproducción
history-empty = Las pistas que reproduzcas aparecerán aquí.
history-not-loaded = No se pudo cargar el historial de reproducción.
history-clear = Vaciar el historial
history-clear-title = Vaciar el historial de reproducción
history-clear-confirm = Se quitarán todas las reproducciones de este dispositivo. Esta acción no se puede deshacer.
filter-library = Filtrar tu biblioteca
filter-album = Filtrar las pistas del álbum
filter-reset = Restablecer los filtros
filter-duration = Duración
filter-year = Año
filter-explicit = Solo explícitas
filter-playable = Solo reproducibles

# view
view-list = Lista
view-cards = Cuadrícula

# toolbar
tool-columns = Columnas
tool-sort = Ordenar
tool-filters = Filtros

# login
login-signed-out = Inicia sesión para cargar tu biblioteca musical
login-restoring = Comprobando tu sesión guardada…
login-authorizing = Esperando la autorización en tu navegador…
login-signed-in = Sesión iniciada como { $name }
login-failed-title = No se pudo iniciar sesión
login-problem-region = Spotify no abre una sesión desde el país en el que estás. Inicia sesión desde tu país de origen o cambia el país en tu cuenta de Spotify.
login-problem-credentials = Tu sesión guardada de Spotify ya no es válida. Inicia sesión de nuevo para continuar.
login-problem-network = Sonora no pudo conectarse con Spotify. Revisa tu conexión a internet e inténtalo de nuevo.
login-problem-cancelled = Cerraste la página del navegador antes de aprobar el inicio de sesión. Empieza de nuevo para terminar.
login-problem-refused = Spotify rechazó el inicio de sesión. Espera un momento e inténtalo de nuevo.
login-problem-premium = Sonora reproduce a través de Spotify Premium y esta cuenta no lo tiene. Inicia sesión con una cuenta Premium para continuar.
login-sign-in = Iniciar sesión con { $provider }
login-connect-cookies = Pegar las cookies manualmente
login-use = Usar { $provider }
login-guest-title = Modo invitado
login-guest-use = Usar el modo invitado
login-guest-detail = Explora y reproduce sin una cuenta. Tu biblioteca, tus favoritos y tus listas quedan fuera de alcance.
login-usage-consent = Ayúdanos a estimar cuánta gente usa Sonora.
login-device-code = Introduce este código en { $url }
login-cookie-submit = Continuar
login-cookie-hint = Pega aquí el encabezado de solicitud Cookie
login-cookie-step-1 = Abre music.youtube.com y comprueba que has iniciado sesión.
login-cookie-step-2 = Pulsa F12, abre la pestaña Red y recarga la página.
login-cookie-step-3 = Selecciona cualquier solicitud llamada "browse" o "next".
login-cookie-step-4 = En Encabezados, busca Cookie dentro de los encabezados de solicitud, haz clic derecho y copia su valor.
login-cookie-step-note = Pega el valor completo abajo: el panel de cookies de la solicitud no basta, porque el valor tiene que llevar SAPISID y __Secure-3PAPISID.
login-cookie-title = Pega tus cookies de YouTube Music para terminar de iniciar sesión
login-account-title = Elige una cuenta
login-account-detail = Esta sesión tiene iniciada la sesión en más de una cuenta de Google. Elige la que debe usar Sonora.

# album and playlist pages
detail-album = Álbum
detail-playlist = Lista de reproducción
detail-play-album = Reproducir álbum
detail-play-playlist = Reproducir lista

# play button
play-pause = Pausar
play-resume = Reanudar
play-loading = Cargando…

# artist page
artist-eyebrow = Artista
artist-monthly-listeners = { $count ->
    [one] { $value } oyente mensual
   *[other] { $value } oyentes mensuales
}
artist-play = Reproducir ahora
artist-follow = Seguir
artist-unfollow = Dejar de seguir
artist-popular = Popular
artist-popular-eyebrow = Descubre a este artista
artist-popular-empty = Todavía no hay nada que reproducir de este artista
artist-popular-more = Ver todo
artist-popular-less = Ver menos
artist-releases = Lanzamientos
artist-releases-more = Ver todo
artist-releases-less = Ver menos
artist-filter-all = Todo
artist-filter-albums = Álbumes
artist-filter-singles = Sencillos
artist-filter-eps = EP

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

# release kinds
release-album = Álbum
release-single = Sencillo
release-compilation = Recopilatorio
release-ep = EP
release-audiobook = Audiolibro
release-podcast = Pódcast
release-meta = { $year } • { $kind }

# home page
home-quick-picks = Selección rápida
home-listen-again = Escuchar de nuevo
home-quick-picks-eyebrow = Empieza por una canción
home-quick-picks-empty = Marca algunas canciones como favoritas y aparecerán aquí

# search page
search-placeholder = ¿Qué quieres escuchar?
search-browse = Explorar todo
genre-empty = Todavía no hay nada que mostrar aquí
search-best-match = Mejor coincidencia
search-no-matches = Sin coincidencias
search-results = Resultados
search-songs = Canciones
search-artists = Artistas
search-albums-playlists = Álbumes y listas
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } canción en la biblioteca
       *[other] { $count } canciones en la biblioteca
    }
kind-song = Canción
kind-artist = Artista
kind-album = Álbum
kind-playlist = Lista de reproducción

# song page
song-eyebrow = Canción
song-play = Reproducir canción
song-view-album = Ver álbum
song-loading = Cargando la información de la canción…
song-about = Sobre esta canción
song-album = Álbum
song-released = Lanzamiento
song-streams = Reproducciones
song-position = Posición
song-label = Sello
song-popularity = Popularidad
song-popularity-value = { $value } %
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

# song languages
language-ar = Árabe
language-de = Alemán
language-en = Inglés
language-es = Español
language-fr = Francés
language-hi = Hindi
language-it = Italiano
language-ja = Japonés
language-ko = Coreano
language-pt = Portugués
language-ru = Ruso
language-tr = Turco
language-uk = Ucraniano
language-zh = Chino
language-zxx = Sin contenido lingüístico

# counts
count-songs =
    { $count ->
        [one] { $count } canción
       *[other] { $count } canciones
    }
count-tracks =
    { $count ->
        [one] { $count } pista
       *[other] { $count } pistas
    }

# dates
date-just-now = Ahora mismo
date-minute-ago = Hace un minuto
date-minutes-ago = Hace { $count } minutos
date-today = Hoy a las { $time }
date-yesterday = Ayer a las { $time }
date-time = { $date }, { $time }
date-full = { $day } { $month } { $year }
month-1 = ene.
month-2 = feb.
month-3 = mar.
month-4 = abr.
month-5 = may.
month-6 = jun.
month-7 = jul.
month-8 = ago.
month-9 = sept.
month-10 = oct.
month-11 = nov.
month-12 = dic.

# settings
settings-tab-general = General
settings-tab-appearance = Apariencia
settings-tab-playback = Reproducción
settings-theme = Tema
settings-theme-detail = Elige la paleta de colores de la aplicación
settings-opacity = Opacidad
settings-opacity-detail = Ajusta la opacidad del fondo de la aplicación
settings-opacity-value = { $percent } %
settings-theme-config = Abrir la configuración
settings-adaptive = Tema adaptativo
settings-adaptive-detail = Tiñe la paleta con la portada del álbum en reproducción
settings-icons = Paquete de iconos
settings-icons-detail = Elige el conjunto de iconos que usa la interfaz
settings-motion = Reducir el movimiento
settings-motion-detail = Omite las animaciones y transiciones de la interfaz
settings-pace = Velocidad de las animaciones
settings-pace-detail = La rapidez con la que se reproducen las animaciones de la interfaz
settings-saver = Ahorro de batería
settings-saver-detail = Limita la tasa de fotogramas de las animaciones mientras Sonora no está en primer plano, a partir del próximo inicio
settings-corners = Esquinas
settings-corners-detail = Cuánto se redondean las superficies y los controles
settings-font = Tamaño del texto
settings-font-detail = Tamaño base del texto; todo lo demás se escala con él
settings-font-value = { $size } px
settings-startup = Mostrar al iniciar
settings-startup-detail = La pantalla con la que se abre Sonora
settings-entries = Secciones de la barra lateral
settings-entries-detail = Las secciones que aparecen en la barra lateral
settings-entries-pick = Elegir las secciones
settings-language = Idioma
settings-language-detail = El idioma que Sonora usa en toda la interfaz
settings-language-system = Sistema
settings-language-search = Buscar un idioma
settings-language-none = No se encontraron idiomas
settings-typeface = Fuente
settings-typeface-detail = La tipografía que Sonora usa en toda la interfaz
settings-typeface-system = Predeterminada
settings-typeface-search = Buscar una fuente
settings-typeface-none = No se encontraron fuentes
settings-window-controls = Controles de la ventana
settings-window-controls-detail = Dibuja minimizar, maximizar y cerrar en la barra de título
settings-controls-side = Lado de los controles
settings-controls-side-detail = El extremo de la barra de título donde se sitúan los controles
settings-normalisation = Normalizar el volumen
settings-normalisation-detail = Mantiene las pistas a un volumen constante
settings-gapless = Reproducción sin pausas
settings-gapless-detail = Encadena una pista con la siguiente sin pausa, tal como se secuenció el álbum
settings-karaoke-lyrics = Letra en karaoke
settings-karaoke-lyrics-detail = Resalta la letra palabra por palabra cuando hay sincronización disponible
settings-romanized-lyrics = Letra romanizada
settings-romanized-lyrics-detail = Muestra la pronunciación generada localmente para los sistemas de escritura elegidos
settings-romanization-writing-systems = Sistemas de escritura
settings-romanization-japanese = Japonés
settings-romanization-chinese = Chino
settings-romanization-korean = Coreano
settings-romanization-cyrillic = Cirílico
settings-romanization-greek = Griego
settings-romanization-arabic = Árabe
settings-romanization-other = Otros sistemas de escritura
settings-advanced = Avanzado
settings-group-accounts = Cuentas
settings-group-library = Biblioteca
settings-group-text = Texto
settings-group-motion = Movimiento
settings-group-title-bar = Barra de título
settings-group-lyrics = Letra
settings-group-project = Proyecto
settings-adaptive-menu = Menú contextual adaptativo
settings-adaptive-menu-detail = Omite las entradas que la fila ya muestra, como el álbum o el artista
settings-accounts = Gestionar las cuentas
settings-accounts-detail = Los servicios desde los que puede reproducir este dispositivo
settings-provider-none = Sin conectar
settings-provider-connected = Conectado
settings-provider-current = Reproduciendo desde este servicio
settings-provider-guest = Reproduciendo como invitado
settings-provider-switch = Cambiar a
settings-sign-out = Cerrar sesión
settings-local-folder = Carpeta de música importada
settings-local-folder-empty = Sin configurar
settings-choose-folder = Elegir carpeta…
settings-rescan = Volver a escanear
settings-clear-folder = Vaciar
settings-tab-about = Acerca de
settings-version = Versión
settings-version-detail = La compilación de sonora que estás usando
settings-license = Licencia
settings-license-detail = Licencia Pública General de GNU, versión 3 o posterior
settings-license-view = Leer la licencia
settings-source = Código fuente
settings-source-detail = El código fuente correspondiente a esta compilación
settings-source-view = Abrir el repositorio
settings-team = Equipo
settings-team-github = GitHub
settings-role-lead-maintainer = Responsable principal
settings-role-maintainer = Responsable
settings-role-contributor = Colaborador
settings-notice = Copyright © 2026 Sonora Contributors. Sonora se distribuye sin ninguna garantía. Es software libre y puedes redistribuirlo según los términos de la Licencia Pública General de GNU, versión 3 o posterior. Sonora es un cliente no oficial y no está afiliado a Spotify AB.

# themes
theme-system = Sistema
theme-dark = Oscuro
theme-light = Claro
theme-midnight = Medianoche
theme-forest = Bosque
theme-ocean = Océano
theme-rose = Rosa
theme-lavender = Lavanda
theme-amber = Ámbar

# corners
corners-square = Rectas
corners-subtle = Sutiles
corners-rounded = Redondeadas
corners-round = Circulares

# motion
motion-system = Sistema
motion-always = Siempre
motion-never = Nunca
pace-slow = Lenta
pace-base = Estándar
pace-quick = Rápida
saver-off = Desactivado
saver-light = Ligero ({ $fps } FPS)
saver-medium = Medio ({ $fps } FPS)
saver-strong = Fuerte ({ $fps } FPS)

toast-playlist-created = Lista creada
toast-playlist-renamed = Lista renombrada
toast-playlist-deleted = Lista eliminada
toast-playlist-added = Lista añadida a tu biblioteca
toast-playlist-removed = Lista quitada de tu biblioteca
toast-playlist-visibility = Se cambió la visibilidad de la lista
toast-track-added = Se añadió a { $name }
toast-track-removed = Se quitó de { $name }
toast-playlist-failed = No se pudo guardar ese cambio
toast-playlist-busy = Todavía hay otro cambio en curso
toast-playlist-signed-out = Inicia sesión para cambiar las listas
toast-queued-track = { $name } se añadió a la cola
toast-next-track = { $name } suena a continuación
toast-queued-album = El álbum se añadió a la cola
toast-next-album = El álbum suena a continuación
toast-queued-playlist = La lista se añadió a la cola
toast-next-playlist = La lista suena a continuación
toast-queued-artist = El artista se añadió a la cola
toast-next-artist = El artista suena a continuación
toast-queue-failed = No se pudo añadir eso a la cola
toast-keys-refused = Spotify no le está dando claves de reproducción a esta cuenta
toast-sign-in-to-play = { $name } solo se reproduce con la sesión iniciada
toast-track-unplayable = No se pudo reproducir { $name }
toast-library-add-failed = No se pudo añadir { $name } a tu biblioteca
toast-library-remove-failed = No se pudo quitar { $name } de tu biblioteca

# lyrics
lyrics-title = Letra
lyrics-idle = Reproduce algo para ver su letra
lyrics-loading = Buscando la letra…
lyrics-missing = No se encontró la letra, ¡lo sentimos!
lyrics-instrumental = Esta canción es instrumental
lyrics-failed = No se pudo conectar con el servicio de letras
lyrics-follow = Volver a seguir la canción
lyrics-source = Letra de { $source }
lyrics-writers = Escrita por { $writers }

update-available = Ya está disponible Sonora { $version }
update-detail = Estás en la versión { $running }. Mira qué ha cambiado o actualiza ahora.
update-detail-notes = Estás en la versión { $running }. Mira qué ha cambiado y luego actualiza Sonora igual que lo instalaste.
update-notes = Novedades
update-now = Actualizar
update-later = Más tarde
update-working = Descargando la actualización…
update-failed = No se pudo instalar la actualización. Inténtalo de nuevo desde la página de versiones.
settings-check-updates = Buscar actualizaciones
settings-check-updates-detail = Pregunta a GitHub una vez al iniciar si hay una versión más reciente. Sonora instala la actualización por su cuenta solo en Windows; en el resto te muestra qué ha cambiado

# tags
tags-edit-title = Editar etiquetas
tags-sheet-song = Canción
tags-sheet-album = Álbum
tags-sheet-details = Detalles
tags-title = Título
tags-artist = Artista
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
toast-tags-saved = Se guardaron las etiquetas de { $name }
toast-tags-failed = No se pudieron guardar las etiquetas
