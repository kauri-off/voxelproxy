#!/bin/bash
# VoxelProxy Lite — Перенаправление диапазона 25560-25570 на указанный адрес

PORT_RANGE="25560:25570"

# Цвета
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

check_root() {
    if [[ $EUID -ne 0 ]]; then
        echo -e "${RED}Ошибка: запустите через sudo${NC}"
        exit 1
    fi
}

validate_addr() {
    if [[ "$1" != *:* ]]; then
        echo -e "${RED}Ошибка: формат должен быть IP:ПОРТ${NC}"
        return 1
    fi
    return 0
}

# Включить перенаправление
enable_proxy() {
    read -rp "Введите адрес выхода (IP:PORT): " dst
    validate_addr "$dst" || return 1

    # Удаляем старые правила, если они были, чтобы не плодить дубли
    disable_proxy_silent

    echo -e "Включение: любой IP [${PORT_RANGE}] → ${CYAN}${dst}${NC}"
    
    iptables -t nat -A OUTPUT -p tcp --match multiport --dports "$PORT_RANGE" -j DNAT --to-destination "$dst"
    echo -e "${GREEN}Прокси активирован.${NC}"
}

get_current_dst() {
    iptables -t nat -S OUTPUT | grep "multiport --dports $PORT_RANGE" | sed 's/.*--to-destination //'
}

# Выключить перенаправление
disable_proxy() {
    local dst
    dst=$(get_current_dst)
    if [[ -z "$dst" ]]; then
        echo -e "${YELLOW}Маршрутов не найдено.${NC}"
        return 1
    fi

    iptables -t nat -D OUTPUT -p tcp --match multiport --dports "$PORT_RANGE" -j DNAT --to-destination "$dst" 2>/dev/null
    echo -e "${GREEN}Прокси отключен.${NC}"
}

disable_proxy_silent() {
    local dst
    dst=$(get_current_dst)
    if [[ -n "$dst" ]]; then
        iptables -t nat -D OUTPUT -p tcp --match multiport --dports "$PORT_RANGE" -j DNAT --to-destination "$dst" 2>/dev/null
    fi
}

# Очистка логов (истории терминала)
clean_history() {
    local script_name=$(basename "$0")
    sed -i "/${script_name}/d" ~/.bash_history 2>/dev/null
    sed -i "/${script_name}/d" ~/.zsh_history 2>/dev/null
    history -c 2>/dev/null
    echo -e "${GREEN}История команд очищена.${NC}"
}

# Меню
check_root
while true; do
    echo -e "\n${BOLD}VoxelProxy Lite${NC} (Порты: $PORT_RANGE)"
    echo "1) Включить (Задать выход)"
    echo "2) Выключить"
    echo "3) Удалить логи (историю)"
    echo "4) Выход"
    read -rp "Выбор: " choice

    case "$choice" in
        1) enable_proxy ;;
        2) disable_proxy ;;
        3) clean_history ;;
        4) exit 0 ;;
        *) echo "Неверный выбор." ;;
    esac
done