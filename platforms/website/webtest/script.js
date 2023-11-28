https://github.com/ElementsProject/lightning/releases/download/v23.05.2/clightning-v23.05.2-Ubuntu-22.04.tar.xz

document.addEventListener('DOMContentLoaded', function() {
    setInterval(loadData, 30000); 
    loadData(); 
});

function loadData() {
    fetch('http://localhost:8000/sample.json')
        .then(response => response.json())
        .then(data => {
            updateTable(data);
        })
        .catch(error => console.error('Error:', error));
}

function updateTable(data) {
    let tbody = document.querySelector('.table tbody');
    tbody.innerHTML = ''; 

    data.forEach(item => {
        let row = tbody.insertRow();
        row.insertCell().textContent = item.formatted_time;
        row.insertCell().textContent = `$${item.estimated_price.toFixed(2)}`;
        row.insertCell().textContent = `$${item.expected_dollar_amount.toFixed(2)}`;
        row.insertCell().textContent = `$${item.stable_receiver_dollar_amount.toFixed(2)}`;
        row.insertCell().innerHTML = item.payment_made ? '✅' : '';
        row.insertCell().textContent = item.risk_score;
    });
}