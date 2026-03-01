#!/bin/bash
# iptables-route.sh — Управление перенаправлением TCP-трафика через iptables
#
# Использование:
#   sudo ./iptables-route.sh                              — интерактивное меню
#   sudo ./iptables-route.sh add    ВХОД:ПОРТ ВЫХОД:ПОРТ — добавить маршрут
#   sudo ./iptables-route.sh remove ВХОД:ПОРТ ВЫХОД:ПОРТ — удалить маршрут
#   sudo ./iptables-route.sh list                         — показать маршруты
#   sudo ./iptables-route.sh clean-history                — очистить историю
#
# Пример:
#   sudo ./iptables-route.sh add 10.0.0.1:25565 192.168.1.1:25565
#
# Принцип работы: Minecraft-клиент подключается к IP входа, ядро Linux
# (через правило iptables NAT OUTPUT) прозрачно перенаправляет соединение
# на IP выхода. Реальный адрес назначения скрыт от клиента.

# ── Настройки ──────────────────────────────────────────────────────────────
ROUTES_FILE="/etc/voxelproxy-routes.conf"

# ── Цвета ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# ── Глобальные переменные для parse_ip_port ─────────────────────────────────
PARSED_IP=""
PARSED_PORT=""

# ═══════════════════════════════════════════════════════════════════════════
# Вспомогательные функции
# ═══════════════════════════════════════════════════════════════════════════

check_dependencies() {
    local missing=0
    for cmd in iptables; do
        if ! command -v "$cmd" &>/dev/null; then
            echo -e "${RED}Ошибка: команда '${cmd}' не найдена.${NC}"
            echo "Установите iptables: apt-get install iptables  или  yum install iptables"
            missing=1
        fi
    done
    [[ $missing -eq 1 ]] && exit 1
}

check_root() {
    if [[ $EUID -ne 0 ]]; then
        echo -e "${RED}Ошибка: скрипт требует прав root.${NC}"
        echo -e "Запустите: ${BOLD}sudo $0 $*${NC}"
        exit 1
    fi
}

# Проверяет формат IP:PORT
# Возвращает 0 если корректно, иначе выводит ошибку и возвращает 1
validate_addr() {
    local addr="$1"
    local label="${2:-адрес}"
    local ip port

    # Проверяем наличие двоеточия
    if [[ "$addr" != *:* ]]; then
        echo -e "${RED}Ошибка: ${label} '${addr}' должен быть в формате IP:ПОРТ${NC}"
        return 1
    fi

    ip="${addr%:*}"
    port="${addr##*:}"

    # Проверка формата IPv4
    if ! echo "$ip" | grep -qE '^([0-9]{1,3}\.){3}[0-9]{1,3}$'; then
        echo -e "${RED}Ошибка: некорректный IP-адрес '${ip}'${NC}"
        return 1
    fi

    # Проверка диапазона октетов
    local IFS='.'
    local -a octets=($ip)
    for octet in "${octets[@]}"; do
        if [[ $octet -gt 255 ]]; then
            echo -e "${RED}Ошибка: октет IP '${octet}' вне диапазона 0-255${NC}"
            return 1
        fi
    done

    # Проверка порта
    if ! [[ "$port" =~ ^[0-9]+$ ]] || [[ $port -lt 1 || $port -gt 65535 ]]; then
        echo -e "${RED}Ошибка: некорректный порт '${port}' (допустимо: 1-65535)${NC}"
        return 1
    fi

    return 0
}

# Разбивает "IP:ПОРТ" на глобальные переменные PARSED_IP и PARSED_PORT
parse_ip_port() {
    PARSED_IP="${1%:*}"
    PARSED_PORT="${1##*:}"
}

# ═══════════════════════════════════════════════════════════════════════════
# Основные функции управления маршрутами
# ═══════════════════════════════════════════════════════════════════════════

# Добавить маршрут: трафик к SRC перенаправляется на DST
# $1 = "SRC_IP:ПОРТ"  $2 = "DST_IP:ПОРТ"
add_route() {
    local src="$1"
    local dst="$2"

    validate_addr "$src" "адрес входа"  || return 1
    validate_addr "$dst" "адрес выхода" || return 1

    # Проверка дублей
    if [[ -f "$ROUTES_FILE" ]] && grep -qF "$src $dst" "$ROUTES_FILE" 2>/dev/null; then
        echo -e "${YELLOW}Предупреждение: маршрут ${src} → ${dst} уже существует.${NC}"
        return 0
    fi

    parse_ip_port "$src"; local src_ip="$PARSED_IP" src_port="$PARSED_PORT"
    parse_ip_port "$dst"; local dst_ip="$PARSED_IP" dst_port="$PARSED_PORT"

    echo -e "Добавление маршрута: ${CYAN}${src_ip}:${src_port}${NC} → ${CYAN}${dst_ip}:${dst_port}${NC}"

    # Правило OUTPUT DNAT: перенаправление локально исходящих соединений.
    # Используется OUTPUT (не PREROUTING), т.к. Minecraft-клиент работает
    # на этом же компьютере и создаёт соединения локально.
    iptables -t nat -A OUTPUT \
        -d "$src_ip" -p tcp --dport "$src_port" \
        -j DNAT --to-destination "${dst_ip}:${dst_port}"

    local ret=$?
    if [[ $ret -ne 0 ]]; then
        echo -e "${RED}Ошибка при добавлении правила iptables (код: ${ret}).${NC}"
        return 1
    fi

    # Создаём файл учёта если не существует
    if [[ ! -f "$ROUTES_FILE" ]]; then
        echo "# VoxelProxy — маршруты iptables (ВХОД:ПОРТ ВЫХОД:ПОРТ)" > "$ROUTES_FILE"
        echo "# Не редактируйте вручную. Используйте iptables-route.sh." >> "$ROUTES_FILE"
    fi

    echo "$src $dst" >> "$ROUTES_FILE"
    echo -e "${GREEN}Маршрут успешно добавлен.${NC}"
}

# Удалить маршрут
# $1 = "SRC_IP:ПОРТ"  $2 = "DST_IP:ПОРТ"
remove_route() {
    local src="$1"
    local dst="$2"

    validate_addr "$src" "адрес входа"  || return 1
    validate_addr "$dst" "адрес выхода" || return 1

    parse_ip_port "$src"; local src_ip="$PARSED_IP" src_port="$PARSED_PORT"
    parse_ip_port "$dst"; local dst_ip="$PARSED_IP" dst_port="$PARSED_PORT"

    echo -e "Удаление маршрута: ${CYAN}${src_ip}:${src_port}${NC} → ${CYAN}${dst_ip}:${dst_port}${NC}"

    # Удаляем правило iptables (-D вместо -A)
    iptables -t nat -D OUTPUT \
        -d "$src_ip" -p tcp --dport "$src_port" \
        -j DNAT --to-destination "${dst_ip}:${dst_port}" 2>/dev/null

    if [[ $? -ne 0 ]]; then
        echo -e "${YELLOW}Предупреждение: правило iptables не найдено (возможно, уже удалено).${NC}"
    fi

    # Удаляем запись из файла учёта (экранируем точки для sed)
    if [[ -f "$ROUTES_FILE" ]]; then
        local escaped_src escaped_dst
        escaped_src="${src//./\\.}"
        escaped_dst="${dst//./\\.}"
        sed -i "/^${escaped_src} ${escaped_dst}$/d" "$ROUTES_FILE"
    fi

    echo -e "${GREEN}Маршрут удалён.${NC}"
}

# Показать активные маршруты
list_routes() {
    echo ""
    echo -e "${BOLD}══════════════════════════════════════════${NC}"
    echo -e "${BOLD}   VoxelProxy — Активные маршруты         ${NC}"
    echo -e "${BOLD}══════════════════════════════════════════${NC}"
    echo ""

    # Читаем файл учёта
    if [[ -f "$ROUTES_FILE" && -s "$ROUTES_FILE" ]]; then
        local i=0
        while IFS= read -r line; do
            # Пропускаем комментарии и пустые строки
            [[ "$line" =~ ^# || -z "$line" ]] && continue
            local src dst
            src=$(echo "$line" | awk '{print $1}')
            dst=$(echo "$line" | awk '{print $2}')
            (( i++ ))
            echo -e "  ${i}. ${CYAN}${src}${NC}  →  ${CYAN}${dst}${NC}"
        done < "$ROUTES_FILE"

        if [[ $i -eq 0 ]]; then
            echo -e "  ${YELLOW}Нет сохранённых маршрутов.${NC}"
        fi
    else
        echo -e "  ${YELLOW}Нет сохранённых маршрутов.${NC}"
    fi

    echo ""
    echo -e "${BOLD}── Активные правила iptables (OUTPUT DNAT) ──${NC}"
    local rules
    rules=$(iptables -t nat -L OUTPUT -n 2>/dev/null | grep -i "DNAT")
    if [[ -n "$rules" ]]; then
        echo "$rules" | while IFS= read -r rule; do
            echo "  $rule"
        done
    else
        echo -e "  ${YELLOW}Правила DNAT не найдены.${NC}"
    fi
    echo ""
}

# ═══════════════════════════════════════════════════════════════════════════
# Очистка истории терминала
# ═══════════════════════════════════════════════════════════════════════════

clean_history() {
    local script_name
    script_name=$(basename "$0")
    local cleaned=0

    echo -e "Очистка упоминаний '${script_name}' из истории терминала..."

    # ── Bash-история ───────────────────────────────────────────────────────
    local bash_hist="${HISTFILE:-$HOME/.bash_history}"
    if [[ -f "$bash_hist" ]]; then
        local before after
        before=$(wc -l < "$bash_hist")
        sed -i "/${script_name}/d" "$bash_hist"
        after=$(wc -l < "$bash_hist")
        local removed=$(( before - after ))
        if [[ $removed -gt 0 ]]; then
            echo -e "  ${GREEN}bash_history: удалено ${removed} строк.${NC}"
            cleaned=1
        else
            echo "  bash_history: упоминаний не найдено."
        fi
    fi

    # ── Zsh-история ────────────────────────────────────────────────────────
    # Zsh хранит расширенный формат: ": timestamp:duration;команда"
    # Удаляем как строку с командой, так и предшествующую ей метку времени
    local zsh_hist="$HOME/.zsh_history"
    if [[ -f "$zsh_hist" ]]; then
        local before after
        before=$(wc -l < "$zsh_hist")
        # Удаляем строки вида ": 123456789:0;./iptables-route.sh ..."
        sed -i "/;.*${script_name}/d" "$zsh_hist"
        # Удаляем обычные строки без метки времени
        sed -i "/${script_name}/d" "$zsh_hist"
        after=$(wc -l < "$zsh_hist")
        local removed=$(( before - after ))
        if [[ $removed -gt 0 ]]; then
            echo -e "  ${GREEN}zsh_history: удалено ${removed} строк.${NC}"
            cleaned=1
        else
            echo "  zsh_history: упоминаний не найдено."
        fi
    fi

    # ── История текущей сессии bash ────────────────────────────────────────
    # Удаляем из памяти текущего процесса bash (влияет только если
    # clean_history вызвана из интерактивного bash-сессии)
    if [[ -n "$BASH_VERSION" ]]; then
        local nums
        nums=$(history | grep "$script_name" | awk '{print $1}' | sort -rn)
        if [[ -n "$nums" ]]; then
            while IFS= read -r num; do
                history -d "$num" 2>/dev/null
            done <<< "$nums"
            echo -e "  ${GREEN}История текущей сессии очищена.${NC}"
            cleaned=1
        fi
    fi

    # ── Итог ───────────────────────────────────────────────────────────────
    echo ""
    if [[ $cleaned -eq 1 ]]; then
        echo -e "${GREEN}Очистка завершена.${NC}"
        echo -e "${YELLOW}Совет: чтобы следующие запуски не попадали в историю,${NC}"
        echo -e "${YELLOW}добавьте пробел перед командой:  ${BOLD} ./iptables-route.sh${NC}"
        echo -e "${YELLOW}(работает если HISTCONTROL содержит 'ignorespace')${NC}"
    else
        echo -e "Упоминания не найдены — история уже чистая."
    fi
}

# ═══════════════════════════════════════════════════════════════════════════
# CLI и меню
# ═══════════════════════════════════════════════════════════════════════════

usage() {
    echo ""
    echo -e "${BOLD}Использование:${NC}"
    echo "  sudo $0                              — интерактивное меню"
    echo "  sudo $0 add    ВХОД:ПОРТ ВЫХОД:ПОРТ  — добавить маршрут"
    echo "  sudo $0 remove ВХОД:ПОРТ ВЫХОД:ПОРТ  — удалить маршрут"
    echo "  sudo $0 list                         — показать маршруты"
    echo "  sudo $0 clean-history                — очистить историю терминала"
    echo ""
    echo -e "${BOLD}Примеры:${NC}"
    echo "  sudo $0 add    10.0.0.1:25565 192.168.1.1:25565"
    echo "  sudo $0 remove 10.0.0.1:25565 192.168.1.1:25565"
    echo "  sudo $0 list"
    echo ""
    exit 1
}

handle_args() {
    local command="$1"
    shift

    case "$command" in
        add)
            if [[ $# -lt 2 ]]; then
                echo -e "${RED}Ошибка: команда 'add' требует два аргумента: ВХОД:ПОРТ ВЫХОД:ПОРТ${NC}"
                usage
            fi
            add_route "$1" "$2"
            ;;
        remove|del|delete|rm)
            if [[ $# -lt 2 ]]; then
                echo -e "${RED}Ошибка: команда 'remove' требует два аргумента: ВХОД:ПОРТ ВЫХОД:ПОРТ${NC}"
                usage
            fi
            remove_route "$1" "$2"
            ;;
        list|show|ls)
            list_routes
            ;;
        clean-history|clean_history|history)
            clean_history
            ;;
        help|-h|--help)
            usage
            ;;
        *)
            echo -e "${RED}Ошибка: неизвестная команда '${command}'.${NC}"
            usage
            ;;
    esac
}

show_menu() {
    while true; do
        echo ""
        echo -e "${BOLD}╔══════════════════════════════════════════╗${NC}"
        echo -e "${BOLD}║   VoxelProxy — Управление маршрутами     ║${NC}"
        echo -e "${BOLD}╚══════════════════════════════════════════╝${NC}"
        echo ""
        echo "  1) Добавить маршрут"
        echo "  2) Удалить маршрут"
        echo "  3) Показать активные маршруты"
        echo "  4) Очистить историю терминала"
        echo "  5) Выход"
        echo ""
        read -rp "  Выберите действие [1-5]: " choice

        case "$choice" in
            1)
                echo ""
                read -rp "  Адрес входа  (IP:ПОРТ, например 10.0.0.1:25565):     " src
                read -rp "  Адрес выхода (IP:ПОРТ, например 192.168.1.1:25565):  " dst
                add_route "$src" "$dst"
                ;;
            2)
                list_routes
                read -rp "  Адрес входа для удаления  (IP:ПОРТ): " src
                read -rp "  Адрес выхода для удаления (IP:ПОРТ): " dst
                remove_route "$src" "$dst"
                ;;
            3)
                list_routes
                ;;
            4)
                clean_history
                ;;
            5)
                echo "Выход."
                exit 0
                ;;
            *)
                echo -e "${YELLOW}  Неверный выбор. Введите число от 1 до 5.${NC}"
                ;;
        esac
    done
}

# ═══════════════════════════════════════════════════════════════════════════
# Точка входа
# ═══════════════════════════════════════════════════════════════════════════

check_dependencies
check_root "$@"

if [[ $# -gt 0 ]]; then
    handle_args "$@"
else
    show_menu
fi
